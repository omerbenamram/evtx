use crate::ChunkOffset;
use crate::binxml::name::{BinXmlName, BinXmlNameLink};
use crate::err::DeserializationResult;
use crate::utils::ByteCursor;

use log::trace;

#[derive(Debug)]
pub struct StringCache {
    /// Dense mapping from chunk offset -> index in `names`.
    ///
    /// Using an indexed table avoids hashing on the hot path (`expand_string_ref` / element name
    /// resolution), which showed up as `BuildHasher::hash_one` in profiles.
    ///
    /// A value of `u32::MAX` means "no entry".
    index_by_offset: Vec<u32>,
    /// All cached names, stored once.
    names: Vec<BinXmlName>,
}

impl StringCache {
    pub fn populate(data: &[u8], offsets: &[ChunkOffset]) -> DeserializationResult<Self> {
        // Offset -> name index table. EVTX chunks are small (~64KiB), so a dense table is cheap.
        // +1 so that indexing `data.len()` (which shouldn't happen) doesn't panic.
        let mut index_by_offset = vec![u32::MAX; data.len().saturating_add(1)];
        let mut names = Vec::with_capacity(offsets.len());

        for &offset in offsets.iter().filter(|&&offset| offset > 0) {
            let mut cursor = ByteCursor::with_pos(data, offset as usize)?;

            loop {
                let string_position = cursor.pos() as ChunkOffset;
                let link = BinXmlNameLink::from_cursor(&mut cursor)?;
                let name = BinXmlName::from_cursor(&mut cursor)?;

                if let Some(slot) = index_by_offset.get_mut(string_position as usize) {
                    if *slot == u32::MAX {
                        let idx = names.len();
                        names.push(name);
                        *slot = idx
                            .try_into()
                            .unwrap_or_else(|_| panic!("StringCache: too many entries"));
                    }
                }

                trace!("\tNext string will be at {:?}", link.next_string);

                match link.next_string {
                    Some(offset) => {
                        if offset == string_position {
                            break;
                        }
                        cursor.set_pos(offset as usize, "next xml string")?;
                    }
                    None => break,
                }
            }
        }

        Ok(StringCache {
            index_by_offset,
            names,
        })
    }

    pub fn get_cached_string(&self, offset: ChunkOffset) -> Option<&BinXmlName> {
        let idx = *self.index_by_offset.get(offset as usize)?;
        if idx == u32::MAX {
            None
        } else {
            self.names.get(idx as usize)
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
}
