use crate::binxml::tokens::read_template_definition;
use crate::err::Result;

use crate::model::deserialized::BinXMLTemplateDefinition;
use crate::Offset;
pub use byteorder::{LittleEndian, ReadBytesExt};

use encoding::EncodingRef;
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

pub type CachedTemplate<'chunk> = BinXMLTemplateDefinition<'chunk>;

#[derive(Debug, Default)]
pub struct TemplateCache<'chunk>(HashMap<Offset, CachedTemplate<'chunk>>);

impl<'chunk> TemplateCache<'chunk> {
    pub fn new() -> Self {
        TemplateCache(HashMap::new())
    }

    pub fn populate(
        data: &'chunk [u8],
        offsets: &[Offset],
        ansi_codec: EncodingRef,
    ) -> Result<Self> {
        let mut cache = HashMap::new();
        let mut cursor = Cursor::new(data);

        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor.seek(SeekFrom::Start(u64::from(*offset)))?;

            let definition = read_template_definition(&mut cursor, None, ansi_codec)?;
            cache.insert(*offset, definition);
        }

        Ok(TemplateCache(cache))
    }

    pub fn get_template(&self, offset: Offset) -> Option<&CachedTemplate<'chunk>> {
        self.0.get(&offset)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
