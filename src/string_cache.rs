
use crate::binxml::name::BinXmlName;
use crate::err::{self, Result};
use crate::Offset;

use snafu::ResultExt;
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

pub type StringHash = u16;

pub type CachedString = (String, StringHash, Offset);

#[derive(Debug, Default)]
pub struct StringCache(HashMap<Offset, CachedString>);

impl StringCache {
    pub fn populate(data: &[u8], offsets: &[Offset]) -> Result<Self> {
        let mut cache = HashMap::new();
        let mut cursor = Cursor::new(data);

        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor
                .seek(SeekFrom::Start(u64::from(*offset)))
                .context(err::IO)?;
            cache.insert(*offset, BinXmlName::from_stream(&mut cursor)?);
        }

        Ok(StringCache(cache))
    }

    pub fn get_string_and_hash(&self, offset: Offset) -> Option<&CachedString> {
        self.0.get(&offset)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
