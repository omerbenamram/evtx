use crate::binxml::name::BinXmlName;
use crate::err::{DeserializationResult, WrappedIoError};
use crate::ChunkOffset;

use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

pub type StringHash = u16;

pub type CachedString = (String, StringHash, ChunkOffset);

#[derive(Debug, Default)]
pub struct StringCache(HashMap<ChunkOffset, CachedString>);

impl StringCache {
    pub fn populate(data: &[u8], offsets: &[ChunkOffset]) -> DeserializationResult<Self> {
        let mut cache = HashMap::new();
        let mut cursor = Cursor::new(data);

        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor
                .seek(SeekFrom::Start(u64::from(*offset)))
                .map_err(|e| {
                    WrappedIoError::io_error_with_message(
                        e,
                        format!(
                            "Failed to seek when trying to read string at offset: {}",
                            offset
                        ),
                        &mut cursor,
                    )
                })?;

            cache.insert(*offset, BinXmlName::from_stream(&mut cursor)?);
        }

        Ok(StringCache(cache))
    }

    pub fn get_string_and_hash(&self, offset: ChunkOffset) -> Option<&CachedString> {
        self.0.get(&offset)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
