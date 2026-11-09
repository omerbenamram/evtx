use crate::ChunkOffset;
use crate::binxml::name::{BinXmlName, read_name_link};
use crate::err::DeserializationResult;
use crate::utils::ByteCursor;

use ahash::AHashMap;
use log::{debug, trace};

#[derive(Debug)]
pub(crate) struct StringCache(AHashMap<ChunkOffset, BinXmlName>);

impl StringCache {
    /// Read the names of the chunk's string table: `offsets` are its buckets, each the offset
    /// of a chain of names.
    pub(crate) fn populate(data: &[u8], offsets: &[ChunkOffset]) -> Self {
        let mut cache = AHashMap::new();

        for &offset in offsets.iter().filter(|&&offset| offset > 0) {
            // The cache only saves decoding a name again: a name that isn't in it is read
            // from the chunk when a record uses it. So a damaged bucket ends its own chain
            // and nothing else, and the records that don't use its names are unaffected.
            if let Err(err) = Self::read_chain(data, offset, &mut cache) {
                debug!("String table chain at offset {offset} is damaged: {err}");
            }
        }

        StringCache(cache)
    }

    /// Cache the names of the chain that starts at `offset`, up to its end or the first name
    /// that can't be read.
    fn read_chain(
        data: &[u8],
        offset: ChunkOffset,
        cache: &mut AHashMap<ChunkOffset, BinXmlName>,
    ) -> DeserializationResult<()> {
        let mut cursor = ByteCursor::with_pos(data, offset as usize)?;

        loop {
            let string_position = cursor.pos() as ChunkOffset;
            let next_string = read_name_link(&mut cursor)?;
            let name = BinXmlName::from_cursor(&mut cursor)?;

            cache.insert(string_position, name);

            trace!("\tNext string will be at {:?}", next_string);

            match next_string {
                // A string read already ends the chain: the strings after it were read with
                // it, and a damaged chain that links back to it would go round forever.
                Some(offset) if !cache.contains_key(&offset) => {
                    cursor.set_pos(offset as usize, "next xml string")?;
                }
                _ => return Ok(()),
            }
        }
    }

    pub(crate) fn get_cached_string(&self, offset: ChunkOffset) -> Option<&BinXmlName> {
        self.0.get(&offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// A string table entry at `offset` of `data`: the offset of the next string, a hash, and
    /// the name, length-prefixed and NUL-terminated.
    fn write_entry(data: &mut [u8], offset: usize, next: u32, name: &str) {
        let mut entry = next.to_le_bytes().to_vec();
        entry.extend(0u16.to_le_bytes());
        entry.extend((name.len() as u16).to_le_bytes());
        entry.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
        entry.extend(0u16.to_le_bytes());
        data[offset..offset + entry.len()].copy_from_slice(&entry);
    }

    #[test]
    fn chains_that_link_back_end() {
        // A links to B, and B back to A; C links to itself.
        let mut data = vec![0; 128];
        write_entry(&mut data, 16, 48, "A");
        write_entry(&mut data, 48, 16, "B");
        write_entry(&mut data, 80, 80, "C");

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || sender.send(StringCache::populate(&data, &[16, 80])));
        let cache = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the walk ends");

        for (offset, name) in [(16, "A"), (48, "B"), (80, "C")] {
            assert_eq!(cache.get_cached_string(offset).unwrap().as_str(), name);
        }
    }

    #[test]
    fn a_damaged_chain_keeps_the_names_read_before_it_and_the_other_chains() {
        let mut data = vec![0; 128];
        // A links past the end of the chunk, and the third bucket starts there too.
        write_entry(&mut data, 16, 4096, "A");
        write_entry(&mut data, 48, 0, "B");
        // D's length runs past the end of the chunk.
        write_entry(&mut data, 80, 0, "D");
        data[86..88].copy_from_slice(&u16::MAX.to_le_bytes());

        let cache = StringCache::populate(&data, &[16, u32::MAX, 80, 48]);

        for (offset, name) in [(16, "A"), (48, "B")] {
            assert_eq!(cache.get_cached_string(offset).unwrap().as_str(), name);
        }
        assert!(cache.get_cached_string(80).is_none());
    }
}
