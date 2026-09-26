use crate::ChunkOffset;
use crate::binxml::name::{BinXmlName, read_name_link};
use crate::err::DeserializationResult;
use crate::utils::ByteCursor;

use ahash::AHashMap;
use log::trace;

#[derive(Debug)]
pub(crate) struct StringCache(AHashMap<ChunkOffset, BinXmlName>);

impl StringCache {
    pub(crate) fn populate(data: &[u8], offsets: &[ChunkOffset]) -> DeserializationResult<Self> {
        let mut cache = AHashMap::new();

        for &offset in offsets.iter().filter(|&&offset| offset > 0) {
            let mut cursor = ByteCursor::with_pos(data, offset as usize)?;

            loop {
                let string_position = cursor.pos() as ChunkOffset;
                let next_string = read_name_link(&mut cursor)?;
                let name = BinXmlName::from_cursor(&mut cursor)?;

                cache.insert(string_position, name);

                trace!("\tNext string will be at {:?}", next_string);

                match next_string {
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

    pub(crate) fn get_cached_string(&self, offset: ChunkOffset) -> Option<&BinXmlName> {
        self.0.get(&offset)
    }
}
