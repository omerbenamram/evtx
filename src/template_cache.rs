use crate::binxml::tokens::read_template_definition_cursor;
use crate::err::DeserializationResult;

use crate::ChunkOffset;
use crate::model::deserialized::BinXMLTemplateDefinition;
use crate::utils::ByteCursor;

use bumpalo::Bump;
use encoding::EncodingRef;
use log::trace;
use std::collections::HashMap;

#[derive(Debug, Copy, Clone)]
pub(crate) enum CompiledTemplateOp {
    FragmentHeader,
    AttributeList,
    OpenStartElement { name_offset: ChunkOffset },
    Attribute { name_offset: ChunkOffset },
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value { token_index: u32 },
    EntityRef { name_offset: ChunkOffset },
    PITarget { name_offset: ChunkOffset },
    PIData { token_index: u32 },
    StartOfStream,
    EndOfStream,
    Substitution {
        substitution_index: u16,
        ignore: bool,
    },
    /// A token we don't have a specialized compiled representation for.
    /// Consumers should fall back to the generic token path using `definition.tokens[token_index]`.
    Unsupported { token_index: u32 },
}

/// Precompiled representation of a template definition for fast streaming expansion.
#[derive(Debug)]
pub(crate) struct CompiledTemplateDefinition {
    pub(crate) ops: Vec<CompiledTemplateOp>,
}

impl CompiledTemplateDefinition {
    fn compile(template: &BinXMLTemplateDefinition<'_>) -> Self {
        let mut ops = Vec::with_capacity(template.tokens.len());

        for (i, t) in template.tokens.iter().enumerate() {
            match t {
                crate::model::deserialized::BinXMLDeserializedTokens::FragmentHeader(_) => {
                    ops.push(CompiledTemplateOp::FragmentHeader);
                }
                crate::model::deserialized::BinXMLDeserializedTokens::AttributeList => {
                    ops.push(CompiledTemplateOp::AttributeList);
                }
                crate::model::deserialized::BinXMLDeserializedTokens::OpenStartElement(elem) => {
                    ops.push(CompiledTemplateOp::OpenStartElement {
                        name_offset: elem.name.offset,
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::Attribute(attr) => {
                    ops.push(CompiledTemplateOp::Attribute {
                        name_offset: attr.name.offset,
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::CloseStartElement => {
                    ops.push(CompiledTemplateOp::CloseStartElement);
                }
                crate::model::deserialized::BinXMLDeserializedTokens::CloseEmptyElement => {
                    ops.push(CompiledTemplateOp::CloseEmptyElement);
                }
                crate::model::deserialized::BinXMLDeserializedTokens::CloseElement => {
                    ops.push(CompiledTemplateOp::CloseElement);
                }
                crate::model::deserialized::BinXMLDeserializedTokens::Value(_) => {
                    ops.push(CompiledTemplateOp::Value {
                        token_index: u32::try_from(i)
                            .unwrap_or_else(|_| panic!("template token index overflow")),
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::EntityRef(entity) => {
                    ops.push(CompiledTemplateOp::EntityRef {
                        name_offset: entity.name.offset,
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::PITarget(name) => {
                    ops.push(CompiledTemplateOp::PITarget {
                        name_offset: name.name.offset,
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::PIData(_) => {
                    ops.push(CompiledTemplateOp::PIData {
                        token_index: u32::try_from(i)
                            .unwrap_or_else(|_| panic!("template token index overflow")),
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::StartOfStream => {
                    ops.push(CompiledTemplateOp::StartOfStream);
                }
                crate::model::deserialized::BinXMLDeserializedTokens::EndOfStream => {
                    ops.push(CompiledTemplateOp::EndOfStream);
                }
                crate::model::deserialized::BinXMLDeserializedTokens::Substitution(desc) => {
                    ops.push(CompiledTemplateOp::Substitution {
                        substitution_index: desc.substitution_index,
                        ignore: desc.ignore,
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::TemplateInstance(_) => {
                    ops.push(CompiledTemplateOp::Unsupported {
                        token_index: u32::try_from(i)
                            .unwrap_or_else(|_| panic!("template token index overflow")),
                    });
                }
                crate::model::deserialized::BinXMLDeserializedTokens::CDATASection
                | crate::model::deserialized::BinXMLDeserializedTokens::CharRef => {
                    ops.push(CompiledTemplateOp::Unsupported {
                        token_index: u32::try_from(i)
                            .unwrap_or_else(|_| panic!("template token index overflow")),
                    });
                }
            }
        }

        CompiledTemplateDefinition { ops }
    }
}

#[derive(Debug)]
pub(crate) struct CachedTemplateEntry<'chunk> {
    pub(crate) definition: BinXMLTemplateDefinition<'chunk>,
    pub(crate) compiled: CompiledTemplateDefinition,
}

#[derive(Debug, Default)]
pub struct TemplateCache<'chunk>(HashMap<ChunkOffset, CachedTemplateEntry<'chunk>>);

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
        // Reserve a minimal baseline; actual number of cached templates may be higher
        // due to chained template buckets.
        let mut cache = HashMap::with_capacity(offsets.len());

        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            let mut cursor = ByteCursor::with_pos(data, *offset as usize)?;

            loop {
                let table_offset = cursor.pos() as ChunkOffset;
                let definition =
                    read_template_definition_cursor(&mut cursor, None, arena, ansi_codec)?;
                let next_template_offset = definition.header.next_template_offset;

                let compiled = CompiledTemplateDefinition::compile(&definition);
                cache.insert(
                    table_offset,
                    CachedTemplateEntry {
                        definition,
                        compiled,
                    },
                );

                trace!("Next template will be at {}", next_template_offset);

                if next_template_offset == 0 || table_offset == next_template_offset {
                    break;
                }

                cursor.set_pos(next_template_offset as usize, "next template")?;
            }
        }

        Ok(TemplateCache(cache))
    }

    pub(crate) fn get_entry(&self, offset: ChunkOffset) -> Option<&CachedTemplateEntry<'chunk>> {
        self.0.get(&offset)
    }

    pub fn get_template(&self, offset: ChunkOffset) -> Option<&BinXMLTemplateDefinition<'chunk>> {
        self.get_entry(offset).map(|e| &e.definition)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
