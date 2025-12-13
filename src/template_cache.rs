use crate::binxml::tokens::read_template_definition_cursor;
use crate::err::DeserializationResult;

use crate::ChunkOffset;
use crate::model::deserialized::BinXMLTemplateDefinition;
use crate::utils::ByteCursor;

use bumpalo::Bump;
use encoding::EncodingRef;
use log::trace;
use std::collections::HashMap;

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
        arena: &'chunk Bump,
        ansi_codec: EncodingRef,
    ) -> DeserializationResult<Self> {
        let mut cache = HashMap::new();

        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            let mut cursor = ByteCursor::with_pos(data, *offset as usize)?;

            loop {
                let table_offset = cursor.pos() as ChunkOffset;
                let definition =
                    read_template_definition_cursor(&mut cursor, None, arena, ansi_codec)?;
                let next_template_offset = definition.header.next_template_offset;

                cache.insert(table_offset, definition);

                trace!("Next template will be at {}", next_template_offset);

                if next_template_offset == 0 || table_offset == next_template_offset {
                    break;
                }

                cursor.set_pos(next_template_offset as usize, "next template")?;
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
