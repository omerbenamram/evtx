use crate::binxml::deserializer::{BinXmlDeserializer, Context};
use crate::binxml::tokens::read_template_definition;
use crate::error::Error;
use crate::evtx_chunk::EvtxChunk;
use crate::guid::Guid;
use crate::model::deserialized::BinXMLTemplateDefinition;
use crate::Offset;
pub use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

pub type CachedTemplate<'a> = (BinXMLTemplateDefinition<'a>);

#[derive(Debug)]
pub struct TemplateCache<'a>(HashMap<Offset, CachedTemplate<'a>>);

impl<'c> TemplateCache<'c> {
    pub fn new() -> Self {
        TemplateCache(HashMap::new())
    }

    pub fn populate(
        &mut self,
        chunk: &EvtxChunk<'c>,
        data: &'c [u8],
        offsets: &[Offset],
    ) -> Result<(), failure::Error> {
        let mut cursor = Cursor::new(data);
        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor.seek(SeekFrom::Start(*offset as u64))?;
            let definition = read_template_definition(&mut cursor, Context::default())?;
            self.0.insert(*offset, definition);
        }

        Ok(())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
