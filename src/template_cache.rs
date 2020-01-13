use crate::binxml::tokens::read_template_definition;
use crate::err::DeserializationResult;

use crate::model::deserialized::BinXMLTemplateDefinition;
use crate::ChunkOffset;
pub use byteorder::{LittleEndian, ReadBytesExt};

use encoding::EncodingRef;
use log::trace;
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};

pub type CachedTemplate<'chunk> = BinXMLTemplateDefinition<'chunk>;

#[derive(Debug, Default)]
pub struct TemplateCache<'chunk>(HashMap<ChunkOffset, CachedTemplate<'chunk>>);

impl<'chunk> TemplateCache<'chunk> {
    pub fn new() -> Self {
        TemplateCache(HashMap::new())
    }

    pub fn populate(
        data: &'chunk [u8],
        offsets: &[ChunkOffset],
        ansi_codec: EncodingRef,
    ) -> DeserializationResult<Self> {
        let mut cache = HashMap::new();
        let mut cursor = Cursor::new(data);
        let cursor_ref = cursor.borrow_mut();

        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            try_seek!(cursor_ref, offset, "first template")?;

            loop {
                let table_offset = cursor_ref.position() as ChunkOffset;
                let definition = read_template_definition(cursor_ref, None, ansi_codec)?;
                let next_template_offset = definition.header.next_template_offset;

                cache.insert(table_offset, definition);

                trace!("Next template will be at {}", next_template_offset);

                if next_template_offset == 0 {
                    break;
                }

                try_seek!(cursor_ref, next_template_offset, "next template")?;
            }
        }

        Ok(TemplateCache(cache))
    }

    pub fn get_template(&self, offset: ChunkOffset) -> Option<&CachedTemplate<'chunk>> {
        self.0.get(&offset)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
