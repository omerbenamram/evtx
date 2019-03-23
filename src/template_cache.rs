use crate::binxml::deserializer::BinXmlDeserializer;
use crate::error::Error;
use crate::evtx_chunk::EvtxChunk;
use crate::guid::Guid;
use crate::model::deserialized::BinXMLTemplateDefinition;
use crate::Offset;
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

pub type TemplateID = u32;
pub type CachedTemplate<'a> = (BinXMLTemplateDefinition<'a>, TemplateID, Offset);

pub struct TemplateCache<'a>(HashMap<Offset, CachedTemplate<'a>>);

impl<'a> TemplateCache<'a> {
    pub fn new() -> Self {
        TemplateCache(HashMap::new())
    }

    pub fn populate(
        &mut self,
        chunk: &EvtxChunk<'a>,
        data: &'a [u8],
        offsets: &[Offset],
    ) -> Result<(), failure::Error> {
        let mut cursor = Cursor::new(data);
        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor.seek(SeekFrom::Start(*offset as u64))?;
            let next_template_offset = try_read!(cursor, u32);
            let template_guid = Guid::from_stream(&mut cursor)
                .map_err(|e| Error::other("Failed to read GUID from stream", cursor.position()))?;
            let data_size = try_read!(cursor, u32);

            let deser =
                BinXmlDeserializer::from_chunk_at_offset(&chunk, u64::from(*offset), data_size);

            self.0
                .insert(*offset, deser.read_template_definition(&mut cursor)?);
        }

        Ok(())
    }
}
