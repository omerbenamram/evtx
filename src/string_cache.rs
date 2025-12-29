use crate::ChunkOffset;
use crate::binxml::name::{BinXmlName, BinXmlNameLink};
use crate::err::DeserializationResult;
use crate::utils::ByteCursor;

use ahash::AHashMap;
use log::trace;

#[derive(Debug)]
pub struct StringCache(AHashMap<ChunkOffset, BinXmlName>);

impl StringCache {
    pub fn populate(data: &[u8], offsets: &[ChunkOffset]) -> DeserializationResult<Self> {
        let mut cache = AHashMap::new();

        for &offset in offsets.iter().filter(|&&offset| offset > 0) {
            let mut cursor = ByteCursor::with_pos(data, offset as usize)?;

            loop {
                let string_position = cursor.pos() as ChunkOffset;
                let link = BinXmlNameLink::from_cursor(&mut cursor)?;
                let name = BinXmlName::from_cursor(&mut cursor)?;

                cache.insert(string_position, name);

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

        Ok(StringCache(cache))
    }

    pub fn get_cached_string(&self, offset: ChunkOffset) -> Option<&BinXmlName> {
        self.0.get(&offset)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
