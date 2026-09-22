use crate::EvtxChunk;
use crate::binxml::ir::RecordContent;
use crate::err::{DeserializationError, DeserializationResult, EvtxError, Result};
use crate::utils::ByteCursor;
use crate::utils::bytes;
use crate::utils::windows::filetime_to_timestamp;

pub use jiff::Timestamp;
#[allow(unused)]
pub use jiff::tz::Offset;

pub type RecordId = u64;

pub(crate) const EVTX_RECORD_HEADER_SIZE: usize = 24;

#[derive(Debug, Clone)]
pub struct EvtxRecord<'a> {
    pub chunk: &'a EvtxChunk<'a>,
    pub event_record_id: RecordId,
    pub timestamp: Timestamp,
    pub(crate) content: RecordContent<'a>,
    pub binxml_offset: u64,
    pub binxml_size: u32,
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
}

impl SerializedEvtxRecord<Vec<u8>> {
    /// Validate the rendered bytes as UTF-8 and convert into a `String` record.
    pub fn into_string_record(self) -> Result<SerializedEvtxRecord<String>> {
        Ok(SerializedEvtxRecord {
            event_record_id: self.event_record_id,
            timestamp: self.timestamp,
            data: String::from_utf8(self.data).map_err(crate::err::SerializationError::from)?,
        })
    }
}

impl EvtxRecordHeader {
    pub fn from_bytes_at(buf: &[u8], offset: usize) -> DeserializationResult<EvtxRecordHeader> {
        let _ = bytes::slice_r(buf, offset, EVTX_RECORD_HEADER_SIZE, "EVTX record header")?;

        let magic = bytes::read_array_r::<4>(buf, offset, "record header magic")?;
        if &magic != b"\x2a\x2a\x00\x00" {
            return Err(DeserializationError::InvalidEvtxRecordHeaderMagic { magic });
        }

        let size = bytes::read_u32_le_r(buf, offset + 4, "record.data_size")?;
        let record_id = bytes::read_u64_le_r(buf, offset + 8, "record.event_record_id")?;
        let filetime = bytes::read_u64_le_r(buf, offset + 16, "record.filetime")?;

        let timestamp = filetime_to_timestamp(filetime)?;

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
        })
    }

    /// Render the record through the compiled-template machinery: cached
    /// program when the record's templates compile (the common case), the
    /// same materialized walker otherwise.
    fn render_into(&self, json: bool, data: &mut Vec<u8>) -> Result<()> {
        use crate::binxml::compiled::{render_tree_json, render_tree_xml, try_render_validated};
        let render = |tree: &crate::model::ir::IrTree<'_>, data: &mut Vec<u8>| {
            if json {
                render_tree_json(tree, &self.chunk.settings, data)
            } else {
                render_tree_xml(tree, &self.chunk.settings, data)
            }
        };
        match &self.content {
            RecordContent::Tree(tree) => render(tree, data),
            RecordContent::Template(content) => {
                let mut caches = self.chunk.render_caches.borrow_mut();
                if try_render_validated(content, self.chunk, &mut caches, json, data) {
                    return Ok(());
                }
                render(&content.materialize(&self.chunk.arena)?, data)
            }
        }
    }

    /// Consumes the record and renders it as compact JSON bytes (no UTF-8 validation pass).
    ///
    /// The renderers only emit valid UTF-8; use this in write-to-sink paths where the
    /// `String` type is not needed.
    pub fn into_json_bytes(self) -> Result<SerializedEvtxRecord<Vec<u8>>> {
        // Estimate buffer size based on BinXML size
        let mut data = Vec::with_capacity(self.binxml_size as usize * 2);
        self.render_into(true, &mut data)
            .map_err(|e| EvtxError::FailedToParseRecord {
                record_id: self.event_record_id,
                source: Box::new(e),
            })?;
        Ok(SerializedEvtxRecord {
            event_record_id: self.event_record_id,
            timestamp: self.timestamp,
            data,
        })
    }

    /// Consumes the record and renders it as compact JSON (streaming IR renderer).
    pub fn into_json(self) -> Result<SerializedEvtxRecord<String>> {
        self.into_json_bytes()?.into_string_record()
    }

    /// Consumes the record and renders it as XML bytes (no UTF-8 validation pass).
    pub fn into_xml_bytes(self) -> Result<SerializedEvtxRecord<Vec<u8>>> {
        let mut data = Vec::with_capacity(self.binxml_size as usize * 2);
        self.render_into(false, &mut data)
            .map_err(|e| EvtxError::FailedToParseRecord {
                record_id: self.event_record_id,
                source: Box::new(e),
            })?;
        Ok(SerializedEvtxRecord {
            event_record_id: self.event_record_id,
            timestamp: self.timestamp,
            data,
        })
    }

    /// Consumes the record and parse it, producing an XML serialized record.
    pub fn into_xml(self) -> Result<SerializedEvtxRecord<String>> {
        self.into_xml_bytes()?.into_string_record()
    }

    /// Parse all `TemplateInstance` substitution arrays from this record.
    ///
    /// This is a lightweight scan over the record's BinXML stream that extracts typed substitution
    /// values without building a legacy token vector.
    pub fn template_instances(&self) -> Result<Vec<crate::binxml::BinXmlTemplateValues<'a>>> {
        use crate::binxml::name::BinXmlNameEncoding;
        use crate::binxml::tokens::{
            read_attribute_cursor, read_entity_ref_cursor, read_fragment_header_cursor,
            read_open_start_element_cursor, read_processing_instruction_data_cursor,
            read_processing_instruction_target_cursor, read_substitution_descriptor_cursor,
            read_template_values_cursor,
        };

        let ansi_codec = self.chunk.settings.get_ansi_codec();
        let mut out: Vec<crate::binxml::BinXmlTemplateValues<'a>> = Vec::new();

        let mut cursor = ByteCursor::with_pos(self.chunk.data, self.binxml_offset as usize)?;
        let mut data_read: u32 = 0;
        let data_size = self.binxml_size;
        let mut eof = false;

        while !eof && data_read < data_size {
            let start = cursor.position();
            let token_byte = cursor.u8()?;

            match token_byte {
                0x00 => {
                    eof = true;
                }
                0x0c => {
                    let template = read_template_values_cursor(
                        &mut cursor,
                        Some(self.chunk),
                        ansi_codec,
                        &self.chunk.arena,
                    )?;
                    out.push(template);
                }
                0x01 => {
                    let _ = read_open_start_element_cursor(
                        &mut cursor,
                        false,
                        false,
                        BinXmlNameEncoding::Offset,
                    )?;
                }
                0x41 => {
                    let _ = read_open_start_element_cursor(
                        &mut cursor,
                        true,
                        false,
                        BinXmlNameEncoding::Offset,
                    )?;
                }
                0x02..=0x04 => {
                    // Structural tokens; no payload.
                }
                0x05 | 0x45 => {
                    let _ = crate::binxml::value_variant::BinXmlValue::from_binxml_cursor_in(
                        &mut cursor,
                        Some(self.chunk),
                        None,
                        ansi_codec,
                        &self.chunk.arena,
                    )?;
                }
                0x06 | 0x46 => {
                    let _ = read_attribute_cursor(&mut cursor, BinXmlNameEncoding::Offset)?;
                }
                0x09 | 0x49 => {
                    let _ = read_entity_ref_cursor(&mut cursor, BinXmlNameEncoding::Offset)?;
                }
                0x0a => {
                    let _ = read_processing_instruction_target_cursor(
                        &mut cursor,
                        BinXmlNameEncoding::Offset,
                    )?;
                }
                0x0b => {
                    let _ = read_processing_instruction_data_cursor(&mut cursor)?;
                }
                0x0d => {
                    let _ = read_substitution_descriptor_cursor(&mut cursor, false)?;
                }
                0x0e => {
                    let _ = read_substitution_descriptor_cursor(&mut cursor, true)?;
                }
                0x0f => {
                    let _ = read_fragment_header_cursor(&mut cursor)?;
                }
                0x07 | 0x47 => {
                    return Err(DeserializationError::UnimplementedToken {
                        name: "CDataSection",
                        offset: cursor.position(),
                    }
                    .into());
                }
                0x08 | 0x48 => {
                    return Err(DeserializationError::UnimplementedToken {
                        name: "CharReference",
                        offset: cursor.position(),
                    }
                    .into());
                }
                _ => {
                    return Err(DeserializationError::InvalidToken {
                        value: token_byte,
                        offset: cursor.position(),
                    }
                    .into());
                }
            }

            let total_read = cursor.position() - start;
            data_read = data_read.saturating_add(total_read as u32);
        }

        Ok(out)
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
        use crate::binxml::ir::{TemplateContent, ValidatedInstance};
        use crate::binxml::value_variant::BinXmlValueType;
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
            value_type: BinXmlValueType::BinXmlType,
            optional: false,
        }));
        let root = template.new_node(root);
        let mut frags = IrArena::new_in(bump);
        let mut child = Element::new_in(Name::new("Child"), bump);
        child.push_child(Node::Text(Text::utf8("validated-fragment")));
        let child = frags.new_node(child);
        record.content = RecordContent::Template(TemplateContent {
            root: ValidatedInstance {
                offset: u32::MAX,
                template: Rc::new(IrTree::new(template, root)),
                has_literal_array: false,
                has_arrays: false,
                values: vec![ValidatedValue::Fragment(child)],
            },
            nested: Vec::new(),
            frags: ManuallyDrop::new(frags),
        });
        // This content has no corresponding wire bytes. XML uses the compiled
        // fragment path; JSON's generic-fragment fallback materializes it.
        assert!(
            record
                .clone()
                .into_xml()
                .unwrap()
                .data
                .contains("validated-fragment")
        );
        assert!(
            record
                .into_json()
                .unwrap()
                .data
                .contains("validated-fragment")
        );
    }

    #[test]
    fn repeated_array_slots_fall_back_in_both_formats() {
        use crate::binxml::compiled::{
            RenderCaches, render_tree_json, render_tree_xml, try_render_validated,
        };
        use crate::binxml::ir::{TemplateContent, ValidatedInstance};
        use crate::binxml::value_variant::BinXmlValueType;
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
            value_type: BinXmlValueType::StringArrayType,
            optional: false,
        });
        // The same array is used in an attribute, ordinary text, and a Data
        // expansion site. Every occurrence must expand, not become "x,y".
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
        let content = TemplateContent {
            root: ValidatedInstance {
                offset: u32::MAX,
                template: Rc::new(IrTree::new(arena, root)),
                has_literal_array: false,
                has_arrays: true,
                values: vec![ValidatedValue::Scalar(BinXmlValue::StringArrayType(items))],
            },
            nested: Vec::new(),
            frags: ManuallyDrop::new(IrArena::new_in(bump)),
        };
        let tree = content.materialize(bump).unwrap();
        let mut caches = RenderCaches::default();
        let compiled = [false, true].map(|json| {
            let mut output = b"prefix".to_vec();
            let compiled = try_render_validated(&content, &chunk, &mut caches, json, &mut output);
            if !compiled {
                assert_eq!(output, b"prefix", "partial output must be rolled back");
                let mut materialized = Vec::new();
                if json {
                    render_tree_json(&tree, &chunk.settings, &mut materialized).unwrap();
                } else {
                    render_tree_xml(&tree, &chunk.settings, &mut materialized).unwrap();
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
