use crate::EvtxChunk;
use crate::binxml::BinXmlTemplateValues;
use crate::binxml::compiled::Budget;
use crate::binxml::ir::RecordContent;
use crate::binxml::tokens::{Stream, TokenSink, walk_tokens};
use crate::err::{DeserializationError, DeserializationResult, EvtxError, Result};
use crate::utils::bytes;
use crate::utils::windows::filetime_to_timestamp;
use std::borrow::Cow;

pub use jiff::Timestamp;
#[allow(unused)]
pub use jiff::tz::Offset;

pub type RecordId = u64;

pub(crate) const EVTX_RECORD_HEADER_SIZE: usize = 24;
pub(crate) const EVTX_RECORD_MAGIC: &[u8; 4] = b"\x2a\x2a\x00\x00";

#[derive(Debug, Clone)]
pub struct EvtxRecord<'a> {
    pub chunk: &'a EvtxChunk<'a>,
    pub event_record_id: RecordId,
    pub timestamp: Timestamp,
    pub(crate) content: RecordContent<'a>,
    /// The nodes it has left to render with, once its content is built (see `ChunkBudget`).
    pub(crate) nodes: usize,
    pub binxml_offset: u64,
    pub binxml_size: u32,
    /// Carved from unallocated chunk space (see `ParserSettings::recover_records`).
    pub recovered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvtxRecordHeader {
    pub data_size: u32,
    pub event_record_id: RecordId,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedEvtxRecord<T> {
    pub event_record_id: RecordId,
    pub timestamp: Timestamp,
    pub data: T,
    /// Carved from unallocated chunk space (see `ParserSettings::recover_records`).
    pub recovered: bool,
}

impl SerializedEvtxRecord<Vec<u8>> {
    /// Validate the rendered bytes as UTF-8 and convert into a `String` record.
    pub fn into_string_record(self) -> Result<SerializedEvtxRecord<String>> {
        Ok(SerializedEvtxRecord {
            event_record_id: self.event_record_id,
            timestamp: self.timestamp,
            data: String::from_utf8(self.data).map_err(crate::err::SerializationError::from)?,
            recovered: self.recovered,
        })
    }
}

impl EvtxRecordHeader {
    pub fn from_bytes_at(buf: &[u8], offset: usize) -> DeserializationResult<EvtxRecordHeader> {
        let _ = bytes::slice_r(buf, offset, EVTX_RECORD_HEADER_SIZE, "EVTX record header")?;

        let magic = bytes::read_array_r::<4>(buf, offset, "record header magic")?;
        if &magic != EVTX_RECORD_MAGIC {
            return Err(DeserializationError::InvalidEvtxRecordHeaderMagic { magic });
        }

        let size = bytes::read_u32_le_r(buf, offset + 4, "record.data_size")?;
        let record_id = bytes::read_u64_le_r(buf, offset + 8, "record.event_record_id")?;
        let filetime = bytes::read_u64_le_r(buf, offset + 16, "record.filetime")?;

        let timestamp = filetime_to_timestamp(filetime)
            .map_err(|_| DeserializationError::InvalidRecordTimestamp { filetime })?;

        Ok(EvtxRecordHeader {
            data_size: size,
            event_record_id: record_id,
            timestamp,
        })
    }

    pub fn from_bytes(buf: &[u8]) -> DeserializationResult<EvtxRecordHeader> {
        Self::from_bytes_at(buf, 0)
    }

    pub fn record_data_size(&self) -> Result<u32> {
        // 24 - record header size
        // 4 - copy of size record size
        let decal = EVTX_RECORD_HEADER_SIZE as u32 + 4;
        if self.data_size < decal {
            return Err(EvtxError::InvalidDataSize {
                length: self.data_size,
                expected: decal,
            });
        }
        Ok(self.data_size - decal)
    }
}

/// Render a materialized tree as JSON or XML, within the record's `budget`.
pub(crate) fn render_tree(
    tree: &crate::model::ir::IrTree<'_>,
    settings: &crate::ParserSettings,
    json: bool,
    budget: &Budget,
    out: &mut Vec<u8>,
) -> Result<()> {
    use crate::binxml::compiled::{JsonProgram, TemplateProgram, XmlProgram};
    if json {
        JsonProgram::render_tree(tree, settings, budget, out)
    } else {
        XmlProgram::render_tree(tree, settings, budget, out)
    }
}

impl<'a> EvtxRecord<'a> {
    /// Consumes the record and returns the rendered JSON as a `serde_json::Value`.
    pub fn into_json_value(self) -> Result<SerializedEvtxRecord<serde_json::Value>> {
        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;
        let record_with_json = self.into_json()?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data: serde_json::from_str(&record_with_json.data)
                .map_err(crate::err::SerializationError::from)?,
            recovered: record_with_json.recovered,
        })
    }

    /// Render the record through the compiled-template machinery: cached
    /// program when the record's templates compile (the common case), the
    /// same materialized walker otherwise. With an `.MTA` file set, the
    /// record's tree gets its `RenderingInfo` and is rendered instead. The
    /// chunk's render budget pays for it (see `ChunkBudget`).
    fn render_into(&self, json: bool, data: &mut Vec<u8>) -> Result<()> {
        let settings = &self.chunk.settings;
        let mta = settings.get_mta();
        let chunk_budget = &self.chunk.render_budget;
        chunk_budget.render(self.nodes, data, |budget, data| {
            let mut tree = match &self.content {
                RecordContent::Tree(tree) => Cow::Borrowed(tree),
                RecordContent::Template(content) => {
                    if mta.is_none()
                        && crate::binxml::compiled::try_render_validated(
                            content,
                            self.chunk,
                            &mut self.chunk.render_caches.borrow_mut(),
                            json,
                            budget,
                            data,
                        )?
                    {
                        return Ok(());
                    }
                    Cow::Owned(content.materialize(&self.chunk.arena, budget)?)
                }
            };
            if let Some(mta) = mta {
                let arena = &self.chunk.arena;
                crate::mta::add_rendering_info(tree.to_mut(), self.event_record_id, mta, arena);
            }
            render_tree(&tree, settings, json, budget, data)
        })
    }

    fn render_bytes(self, json: bool) -> Result<SerializedEvtxRecord<Vec<u8>>> {
        // Estimate buffer size based on BinXML size
        let mut data = Vec::with_capacity(self.binxml_size as usize * 2);
        self.render_into(json, &mut data)
            .map_err(|e| EvtxError::record(self.event_record_id, e))?;
        Ok(SerializedEvtxRecord {
            event_record_id: self.event_record_id,
            timestamp: self.timestamp,
            data,
            recovered: self.recovered,
        })
    }

    /// Consumes the record and renders it as compact JSON bytes (no UTF-8 validation pass).
    ///
    /// The renderers only emit valid UTF-8; use this in write-to-sink paths where the
    /// `String` type is not needed.
    pub fn into_json_bytes(self) -> Result<SerializedEvtxRecord<Vec<u8>>> {
        self.render_bytes(true)
    }

    /// Consumes the record and renders it as compact JSON (streaming IR renderer).
    pub fn into_json(self) -> Result<SerializedEvtxRecord<String>> {
        self.into_json_bytes()?.into_string_record()
    }

    /// Consumes the record and renders it as XML bytes (no UTF-8 validation pass).
    pub fn into_xml_bytes(self) -> Result<SerializedEvtxRecord<Vec<u8>>> {
        self.render_bytes(false)
    }

    /// Consumes the record and parse it, producing an XML serialized record.
    pub fn into_xml(self) -> Result<SerializedEvtxRecord<String>> {
        self.into_xml_bytes()?.into_string_record()
    }

    /// Parse all `TemplateInstance` substitution arrays from this record.
    ///
    /// This is a lightweight scan over the record's BinXML stream that extracts typed substitution
    /// values without building a legacy token vector.
    pub fn template_instances(&self) -> Result<Vec<BinXmlTemplateValues<'a>>> {
        struct Instances<'a>(Vec<BinXmlTemplateValues<'a>>);
        impl<'a> TokenSink<'a> for Instances<'a> {
            fn template_instance(&mut self, template: BinXmlTemplateValues<'a>) -> Result<()> {
                self.0.push(template);
                Ok(())
            }
        }

        let mut out = Instances(Vec::new());
        walk_tokens(
            Stream::chunk(self.chunk, &self.chunk.arena),
            self.binxml_offset as usize,
            self.binxml_size as usize,
            false,
            &mut out,
        )?;
        Ok(out.0)
    }
}

#[cfg(test)]
mod validated_content_tests {
    use super::*;
    use crate::binxml::ir::ValidatedValue;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::evtx_parser::{EVTX_CHUNK_SIZE, EVTX_FILE_HEADER_SIZE};
    use crate::{EvtxChunkData, ParserSettings};
    use std::sync::Arc;

    fn first_chunk() -> EvtxChunkData {
        let file = include_bytes!("../samples/security.evtx");
        let chunk = &file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE];
        EvtxChunkData::new(chunk.to_vec(), false).unwrap()
    }

    #[test]
    fn rendering_uses_validated_values_in_both_paths() {
        for separate in [false, true] {
            let mut data = first_chunk();
            let mut chunk = data
                .parse(Arc::new(
                    ParserSettings::default().separate_json_attributes(separate),
                ))
                .unwrap();
            let mut record = chunk.iter().next().unwrap().unwrap();
            let RecordContent::Template(content) = &mut record.content else {
                panic!("template fixture")
            };
            let value = content
                .root
                .values
                .iter_mut()
                .find(|value| matches!(value, ValidatedValue::Scalar(BinXmlValue::StringType(_))))
                .unwrap();
            // Changing only the retained value proves that neither compiled
            // rendering nor the separate-attributes fallback re-reads bytes.
            *value = ValidatedValue::Scalar(BinXmlValue::AnsiStringType("validated-substitution"));
            assert!(
                record
                    .clone()
                    .into_json()
                    .unwrap()
                    .data
                    .contains("validated-substitution")
            );
            assert!(
                record
                    .into_xml()
                    .unwrap()
                    .data
                    .contains("validated-substitution")
            );
        }
    }

    #[test]
    fn rendering_reuses_validated_fragments() {
        use crate::binxml::ir::{TemplateContent, TemplateStats, ValidatedInstance};
        use crate::model::ir::{Element, IrArena, IrTree, Name, Node, Placeholder, Text};
        use std::{mem::ManuallyDrop, rc::Rc};

        let mut data = first_chunk();
        let mut chunk = data.parse(Arc::new(ParserSettings::default())).unwrap();
        let mut record = chunk.iter().next().unwrap().unwrap();
        let bump = &record.chunk.arena;
        let mut template = IrArena::new_in(bump);
        let mut root = Element::new_in(Name::new("Root"), bump);
        root.push_child(Node::Placeholder(Placeholder {
            id: 0,
            optional: false,
        }));
        let root = template.new_node(root);
        let mut frags = IrArena::new_in(bump);
        let mut child = Element::new_in(Name::new("Child"), bump);
        child.push_child(Node::Text(Text::utf8("validated-fragment")));
        let child = frags.new_node(child);
        // A fragment can hold several elements; all of them render.
        let mut sibling = Element::new_in(Name::new("Sibling"), bump);
        sibling.push_child(Node::Text(Text::utf8("second-root")));
        let sibling = frags.new_node(sibling);
        let template = Rc::new(IrTree::new(template, root));
        record.content = RecordContent::Template(TemplateContent {
            root: ValidatedInstance {
                offset: u32::MAX,
                stats: TemplateStats::of(&template).unwrap(),
                template,
                has_arrays: false,
                values: vec![ValidatedValue::Fragment(
                    bump.alloc_slice_copy(&[child, sibling]),
                )],
            },
            nested: Vec::new(),
            frags: ManuallyDrop::new(frags),
        });
        // This content has no corresponding wire bytes. XML uses the compiled
        // fragment path; JSON's generic-fragment fallback materializes it.
        let xml = record.clone().into_xml().unwrap().data;
        assert!(xml.contains("validated-fragment") && xml.contains("second-root"));
        let json = record.into_json().unwrap().data;
        assert!(json.contains("validated-fragment") && json.contains("second-root"));
    }

    #[test]
    fn repeated_array_slots_fall_back_in_both_formats() {
        use crate::binxml::compiled::{
            Budget, JsonProgram, RenderCaches, TemplateProgram, XmlProgram, try_render_validated,
        };
        use crate::binxml::ir::{TemplateContent, TemplateStats, ValidatedInstance};
        use crate::model::ir::{Attr, Element, IrArena, IrTree, IrVec, Name, Node, Placeholder};
        use crate::utils::Utf16LeSlice;
        use std::{mem::ManuallyDrop, rc::Rc};

        let mut data = first_chunk();
        let chunk = data
            .parse(Arc::new(ParserSettings::default().indent(false)))
            .unwrap();
        let bump = &chunk.arena;
        let mut arena = IrArena::new_in(bump);
        let mut root = Element::new_in(Name::new("EventData"), bump);
        let slot = Node::Placeholder(Placeholder {
            id: 0,
            optional: false,
        });
        // The same array is used in an attribute, ordinary text, and a Data
        // expansion site. Every occurrence must expand, not become "x,y": the
        // compiled lanes don't take templates that use a value twice.
        for name in ["Sub", "Other", "Data"] {
            let mut element = Element::new_in(Name::new(name), bump);
            if name == "Sub" {
                let mut value = IrVec::new_in(bump);
                value.push(slot.clone());
                element.attrs.push(Attr {
                    name: Name::new("a"),
                    value,
                });
            } else {
                element.push_child(slot.clone());
            }
            root.push_child(Node::Element(arena.new_node(element)));
        }
        let root = arena.new_node(root);
        let items =
            bump.alloc_slice_copy(&[Utf16LeSlice::new(b"x\0", 1), Utf16LeSlice::new(b"y\0", 1)]);
        let template = Rc::new(IrTree::new(arena, root));
        let content = TemplateContent {
            root: ValidatedInstance {
                offset: u32::MAX,
                stats: TemplateStats::of(&template).unwrap(),
                template,
                has_arrays: true,
                values: vec![ValidatedValue::Scalar(BinXmlValue::StringArrayType(items))],
            },
            nested: Vec::new(),
            frags: ManuallyDrop::new(IrArena::new_in(bump)),
        };
        let tree = content.materialize(bump, &Budget::new(0)).unwrap();
        let mut caches = RenderCaches::default();
        let compiled = [false, true].map(|json| {
            let mut output = b"prefix".to_vec();
            let budget = Budget::new(output.len());
            let compiled =
                try_render_validated(&content, &chunk, &mut caches, json, &budget, &mut output)
                    .unwrap();
            if !compiled {
                assert_eq!(output, b"prefix", "partial output must be rolled back");
                let mut materialized = Vec::new();
                let budget = Budget::new(0);
                if json {
                    JsonProgram::render_tree(&tree, &chunk.settings, &budget, &mut materialized)
                        .unwrap();
                } else {
                    XmlProgram::render_tree(&tree, &chunk.settings, &budget, &mut materialized)
                        .unwrap();
                }
                assert!(!String::from_utf8(materialized).unwrap().contains("x,y"));
            }
            compiled
        });
        assert_eq!(compiled, [false, false], "XML and JSON must materialize");
    }

    #[test]
    fn retained_records_render_after_other_records_and_in_both_formats() {
        let mut data = first_chunk();
        let mut chunk = data.parse(Arc::new(ParserSettings::default())).unwrap();
        let records: Vec<_> = chunk.iter().take(12).map(Result::unwrap).collect();
        let expected: Vec<_> = records
            .iter()
            .map(|record| {
                (
                    record.clone().into_json().unwrap().data,
                    record.clone().into_xml().unwrap().data,
                )
            })
            .collect();
        for (record, (json, xml)) in records.into_iter().zip(expected).rev() {
            assert_eq!(record.clone().into_xml().unwrap().data, xml);
            assert_eq!(record.into_json().unwrap().data, json);
        }
    }

    #[test]
    fn invalid_substitution_fails_before_record_is_yielded() {
        let mut data = first_chunk();
        let mut payload = vec![0x0c, 0];
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&u32::MAX.to_le_bytes()); // unused definition
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&[8, 0, 0x11, 0]); // FILETIME descriptor
        payload.extend_from_slice(&u64::MAX.to_le_bytes()); // invalid timestamp
        let size = (24 + payload.len() + 4) as u32;
        data.data[512 + 4..512 + 8].copy_from_slice(&size.to_le_bytes());
        data.data[512 + 24..512 + 24 + payload.len()].copy_from_slice(&payload);
        let mut chunk = data.parse(Arc::new(ParserSettings::default())).unwrap();
        let error = chunk.iter().next().unwrap().unwrap_err();
        let EvtxError::FailedToParseRecord { source, .. } = error else {
            panic!("{error:?}")
        };
        assert!(matches!(
            *source,
            EvtxError::DeserializationError(DeserializationError::InvalidDateTimeError)
        ));
    }
}
