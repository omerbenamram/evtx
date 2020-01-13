use crate::binxml::name::{BinXmlName, BinXmlNameLink};
use crate::err::DeserializationResult;
use crate::ChunkOffset;

use log::trace;
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

#[derive(Debug)]
pub struct StringCache(HashMap<ChunkOffset, BinXmlName>);

impl StringCache {
    pub fn populate(data: &[u8], offsets: &[ChunkOffset]) -> DeserializationResult<Self> {
        let mut cache = HashMap::new();
        let mut cursor = Cursor::new(data);
        let cursor_ref = cursor.borrow_mut();

        for &offset in offsets.iter().filter(|&&offset| offset > 0) {
            try_seek!(cursor_ref, offset, "first xml string")?;

            loop {
                let string_position = cursor_ref.position() as ChunkOffset;
                let link = BinXmlNameLink::from_stream(cursor_ref)?;
                let name = BinXmlName::from_stream(cursor_ref)?;

                cache.insert(string_position, name);

                trace!("\tNext string will be at {:?}", link.next_string);

                match link.next_string {
                    Some(offset) => {
                        try_seek!(cursor_ref, offset, "next xml string")?;
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
