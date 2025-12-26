use crate::ChunkOffset;
use crate::binxml::name::{BinXmlName, BinXmlNameLink};
use crate::err::DeserializationResult;
use crate::utils::ReadExt;

use log::trace;
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::io::Cursor;

#[derive(Debug)]
pub struct StringCache(HashMap<ChunkOffset, BinXmlName>);

impl StringCache {
    pub fn populate(data: &[u8], offsets: &[ChunkOffset]) -> DeserializationResult<Self> {
        let mut cache = HashMap::new();
        let mut cursor = Cursor::new(data);
        let cursor_ref = cursor.borrow_mut();

        for &offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor_ref.try_seek_abs_named(u64::from(offset), "first xml string")?;

            loop {
                let string_position = cursor_ref.position() as ChunkOffset;
                let link = BinXmlNameLink::from_stream(cursor_ref)?;
                let name = BinXmlName::from_stream(cursor_ref)?;

                cache.insert(string_position, name);

                trace!("\tNext string will be at {:?}", link.next_string);

                match link.next_string {
                    Some(offset) => {
                        if offset == string_position {
                            break;
                        }
                        cursor_ref.try_seek_abs_named(u64::from(offset), "next xml string")?;
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
