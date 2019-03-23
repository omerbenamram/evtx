use crate::binxml::name::BinXmlName;
use crate::Offset;
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

pub type StringHash = u16;

pub type CachedString = (String, StringHash, Offset);

#[derive(Debug)]
pub struct StringCache(HashMap<Offset, CachedString>);

impl StringCache {
    pub fn new() -> Self {
        StringCache(HashMap::new())
    }

    pub fn populate(&mut self, data: &[u8], offsets: &[Offset]) -> Result<(), failure::Error> {
        let mut cursor = Cursor::new(data);

        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor.seek(SeekFrom::Start(*offset as u64))?;
            self.0
                .insert(*offset, BinXmlName::from_stream(&mut cursor)?);
        }

        Ok(())
    }

    pub fn get_string_and_hash(&self, offset: Offset) -> Option<&CachedString> {
        self.0.get(&offset)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
