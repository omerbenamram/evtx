//! Extract WEVT_TEMPLATE resources from PE files.
//!
//! This is primarily intended to support building an offline cache of EVTX templates
//! (see `omerbenamram/evtx` issue #103).

use encoding::EncodingRef;
use thiserror::Error;
use winstructs::guid::Guid;

pub mod manifest;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceIdentifier {
    Id(u32),
    Name(String),
}

#[derive(Debug, Clone)]
pub struct WevtTemplateResource {
    /// The second-level entry under the `WEVT_TEMPLATE` resource type (often `1`).
    pub resource: ResourceIdentifier,
    /// Language ID associated with this resource data.
    pub lang_id: u32,
    /// Raw resource bytes (typically starts with `CRIM|K\0\0`).
    pub data: Vec<u8>,
}

// === Parsing of WEVT_TEMPLATE payloads (CRIM/WEVT/TTBL/TEMP) ===
//
// Primary references:
// - MS-EVEN6 BinXml grammar (inline names): `Name = NameHash NameNumChars NullTerminatedUnicodeString`
//   and token layouts for OpenStartElement/Attribute/EntityRef/PITarget.
// - libfwevt docs: "Windows Event manifest binary format" (WEVT_TEMPLATE / CRIM / WEVT / TTBL / TEMP layouts).

#[derive(Debug, Clone)]
pub struct WevtTempTemplateHeader {
    /// Number of template item descriptors.
    pub item_descriptor_count: u32,
    /// Number of template item names.
    pub item_name_count: u32,
    /// Template items offset (relative to the start of the CRIM blob).
    pub template_items_offset: u32,
    /// Unknown; libfwevt suggests this correlates with the template kind (e.g. EventData vs UserData).
    pub event_type: u32,
    /// Template GUID.
    pub guid: Guid,
}

#[derive(Debug, Clone)]
pub struct WevtTempTemplateRef {
    /// Offset of the containing `TTBL` within the resource blob.
    pub ttbl_offset: u32,
    /// Offset of this `TEMP` structure within the resource blob.
    pub temp_offset: u32,
    /// Total size of this `TEMP` structure, in bytes.
    pub temp_size: u32,
    pub header: WevtTempTemplateHeader,
}

#[derive(Debug, Error)]
pub enum WevtTemplateExtractError {
    #[error("input is not a valid PE file: {message}")]
    InvalidPe { message: &'static str },

    #[error("malformed PE file: {message}")]
    MalformedPe { message: &'static str },

    #[error("failed to map RVA 0x{rva:08x} to a file offset")]
    UnmappedRva { rva: u32 },

    #[error("resource directory is malformed: {message}")]
    MalformedResource { message: &'static str },

    #[error("failed to decode UTF-16 resource name")]
    InvalidResourceName,
}

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_ptr: u32,
    raw_size: u32,
}

struct PeView<'a> {
    bytes: &'a [u8],
    sections: Vec<Section>,
    rsrc_rva: u32,
    rsrc_size: u32,
}

impl<'a> PeView<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Option<Self>, WevtTemplateExtractError> {
        if bytes.len() < 0x40 {
            return Err(WevtTemplateExtractError::InvalidPe {
                message: "file too small",
            });
        }
        if &bytes[0..2] != b"MZ" {
            return Err(WevtTemplateExtractError::InvalidPe {
                message: "missing MZ header",
            });
        }

        let e_lfanew = read_u32(bytes, 0x3c).ok_or(WevtTemplateExtractError::MalformedPe {
            message: "missing e_lfanew",
        })? as usize;

        let pe_sig_end = e_lfanew
            .checked_add(4)
            .ok_or(WevtTemplateExtractError::MalformedPe {
                message: "e_lfanew overflow",
            })?;

        if pe_sig_end > bytes.len() {
            return Err(WevtTemplateExtractError::MalformedPe {
                message: "e_lfanew out of bounds",
            });
        }
        if &bytes[e_lfanew..pe_sig_end] != b"PE\0\0" {
            return Err(WevtTemplateExtractError::InvalidPe {
                message: "missing PE signature",
            });
        }

        let coff_offset = pe_sig_end;
        let number_of_sections =
            read_u16(bytes, coff_offset + 2).ok_or(WevtTemplateExtractError::MalformedPe {
                message: "missing COFF number_of_sections",
            })? as usize;
        let size_of_optional_header =
            read_u16(bytes, coff_offset + 16).ok_or(WevtTemplateExtractError::MalformedPe {
                message: "missing COFF size_of_optional_header",
            })? as usize;

        let optional_header_offset = coff_offset + 20;
        let section_headers_offset = optional_header_offset
            .checked_add(size_of_optional_header)
            .ok_or(WevtTemplateExtractError::MalformedPe {
                message: "optional header overflow",
            })?;

        if section_headers_offset > bytes.len() {
            return Err(WevtTemplateExtractError::MalformedPe {
                message: "optional header out of bounds",
            });
        }

        let optional_magic = read_u16(bytes, optional_header_offset).ok_or(
            WevtTemplateExtractError::MalformedPe {
                message: "missing optional header magic",
            },
        )?;

        let (number_of_rva_and_sizes_offset, data_directories_offset) = match optional_magic {
            0x10b => (optional_header_offset + 92, optional_header_offset + 96), // PE32
            0x20b => (optional_header_offset + 108, optional_header_offset + 112), // PE32+
            _ => {
                return Err(WevtTemplateExtractError::InvalidPe {
                    message: "unsupported optional header magic",
                });
            }
        };

        let number_of_rva_and_sizes = read_u32(bytes, number_of_rva_and_sizes_offset).ok_or(
            WevtTemplateExtractError::MalformedPe {
                message: "missing number_of_rva_and_sizes",
            },
        )?;

        // Resource table is IMAGE_DIRECTORY_ENTRY_RESOURCE = 2
        if number_of_rva_and_sizes <= 2 {
            return Ok(None);
        }

        let rsrc_entry_offset = data_directories_offset.checked_add(2 * 8).ok_or(
            WevtTemplateExtractError::MalformedPe {
                message: "data directory offset overflow",
            },
        )?;

        let rsrc_rva =
            read_u32(bytes, rsrc_entry_offset).ok_or(WevtTemplateExtractError::MalformedPe {
                message: "missing resource table RVA",
            })?;
        let rsrc_size = read_u32(bytes, rsrc_entry_offset + 4).ok_or(
            WevtTemplateExtractError::MalformedPe {
                message: "missing resource table size",
            },
        )?;

        if rsrc_rva == 0 || rsrc_size == 0 {
            return Ok(None);
        }

        let mut sections = Vec::with_capacity(number_of_sections);
        let mut off = section_headers_offset;
        for _ in 0..number_of_sections {
            if off + 40 > bytes.len() {
                return Err(WevtTemplateExtractError::MalformedPe {
                    message: "section headers out of bounds",
                });
            }

            let virtual_size =
                read_u32(bytes, off + 8).ok_or(WevtTemplateExtractError::MalformedPe {
                    message: "missing section virtual_size",
                })?;
            let virtual_address =
                read_u32(bytes, off + 12).ok_or(WevtTemplateExtractError::MalformedPe {
                    message: "missing section virtual_address",
                })?;
            let raw_size =
                read_u32(bytes, off + 16).ok_or(WevtTemplateExtractError::MalformedPe {
                    message: "missing section raw_size",
                })?;
            let raw_ptr =
                read_u32(bytes, off + 20).ok_or(WevtTemplateExtractError::MalformedPe {
                    message: "missing section raw_ptr",
                })?;

            sections.push(Section {
                virtual_address,
                virtual_size,
                raw_ptr,
                raw_size,
            });

            off += 40;
        }

        Ok(Some(PeView {
            bytes,
            sections,
            rsrc_rva,
            rsrc_size,
        }))
    }

    fn rva_to_file_offset(&self, rva: u32) -> Option<usize> {
        for s in &self.sections {
            let start = s.virtual_address;
            let end = start.saturating_add(s.virtual_size.max(s.raw_size));
            if rva >= start && rva < end {
                let delta = rva - start;
                return Some(s.raw_ptr as usize + delta as usize);
            }
        }
        None
    }

    fn read_rva(&self, rva: u32, size: usize) -> Result<&'a [u8], WevtTemplateExtractError> {
        let file_offset = self
            .rva_to_file_offset(rva)
            .ok_or(WevtTemplateExtractError::UnmappedRva { rva })?;
        let end = file_offset
            .checked_add(size)
            .ok_or(WevtTemplateExtractError::MalformedPe {
                message: "RVA read overflow",
            })?;
        if end > self.bytes.len() {
            return Err(WevtTemplateExtractError::MalformedPe {
                message: "RVA read out of bounds",
            });
        }
        Ok(&self.bytes[file_offset..end])
    }
}

struct ResourceSection<'a> {
    buf: &'a [u8],
}

impl<'a> ResourceSection<'a> {
    fn from_pe(pe: &'a PeView<'a>) -> Result<Self, WevtTemplateExtractError> {
        let buf = pe.read_rva(pe.rsrc_rva, pe.rsrc_size as usize)?;
        Ok(ResourceSection { buf })
    }

    fn read_u16(&self, offset: usize) -> Result<u16, WevtTemplateExtractError> {
        read_u16(self.buf, offset).ok_or(WevtTemplateExtractError::MalformedResource {
            message: "read_u16 out of bounds",
        })
    }

    fn read_u32(&self, offset: usize) -> Result<u32, WevtTemplateExtractError> {
        read_u32(self.buf, offset).ok_or(WevtTemplateExtractError::MalformedResource {
            message: "read_u32 out of bounds",
        })
    }

    fn read_buf(&self, offset: usize, len: usize) -> Result<&'a [u8], WevtTemplateExtractError> {
        let end = offset
            .checked_add(len)
            .ok_or(WevtTemplateExtractError::MalformedResource {
                message: "read_buf overflow",
            })?;
        if end > self.buf.len() {
            return Err(WevtTemplateExtractError::MalformedResource {
                message: "read_buf out of bounds",
            });
        }
        Ok(&self.buf[offset..end])
    }

    fn read_name(&self, offset: usize) -> Result<String, WevtTemplateExtractError> {
        let char_count = self.read_u16(offset)? as usize;
        let buf = self.read_buf(offset + 2, char_count * 2)?;
        let mut chars = Vec::with_capacity(char_count);
        for i in 0..char_count {
            let c = read_u16(buf, i * 2).ok_or(WevtTemplateExtractError::MalformedResource {
                message: "resource name read out of bounds",
            })?;
            chars.push(c);
        }
        String::from_utf16(&chars).map_err(|_e| WevtTemplateExtractError::InvalidResourceName)
    }
}

struct ResourceNodeHeader {
    named_entry_count: u16,
    id_entry_count: u16,
}

impl ResourceNodeHeader {
    fn read(rsrc: &ResourceSection<'_>, offset: usize) -> Result<Self, WevtTemplateExtractError> {
        Ok(ResourceNodeHeader {
            // skip 0..12
            named_entry_count: rsrc.read_u16(offset + 12)?,
            id_entry_count: rsrc.read_u16(offset + 14)?,
        })
    }
}

#[derive(Clone, Copy)]
struct ResourceNodeEntry {
    id: u32,
    offset: u32,
}

impl ResourceNodeEntry {
    fn read(rsrc: &ResourceSection<'_>, offset: usize) -> Result<Self, WevtTemplateExtractError> {
        Ok(ResourceNodeEntry {
            id: rsrc.read_u32(offset)?,
            offset: rsrc.read_u32(offset + 4)?,
        })
    }

    fn has_name(self) -> bool {
        (self.id & 0x8000_0000) != 0
    }

    fn is_dir(self) -> bool {
        (self.offset & 0x8000_0000) != 0
    }

    fn id_value(self) -> u32 {
        self.id & 0x7FFF_FFFF
    }

    fn child_offset(self) -> usize {
        (self.offset & 0x7FFF_FFFF) as usize
    }

    fn identifier(
        self,
        rsrc: &ResourceSection<'_>,
    ) -> Result<ResourceIdentifier, WevtTemplateExtractError> {
        if self.has_name() {
            let name_offset = self.id_value() as usize;
            Ok(ResourceIdentifier::Name(rsrc.read_name(name_offset)?))
        } else {
            Ok(ResourceIdentifier::Id(self.id_value()))
        }
    }

    fn child(self, rsrc: &ResourceSection<'_>) -> Result<NodeChild, WevtTemplateExtractError> {
        let off = self.child_offset();
        if self.is_dir() {
            Ok(NodeChild::Node(ResourceNode::read(rsrc, off)?))
        } else {
            Ok(NodeChild::Data(ResourceDataDescriptor::read(rsrc, off)?))
        }
    }
}

struct ResourceNode {
    entries: Vec<ResourceNodeEntry>,
}

impl ResourceNode {
    fn read(rsrc: &ResourceSection<'_>, offset: usize) -> Result<Self, WevtTemplateExtractError> {
        let header = ResourceNodeHeader::read(rsrc, offset)?;

        let mut entries =
            Vec::with_capacity(header.named_entry_count as usize + header.id_entry_count as usize);
        let mut off = offset + 16;

        for _ in 0..header.named_entry_count {
            entries.push(ResourceNodeEntry::read(rsrc, off)?);
            off += 8;
        }
        for _ in 0..header.id_entry_count {
            entries.push(ResourceNodeEntry::read(rsrc, off)?);
            off += 8;
        }

        Ok(ResourceNode { entries })
    }

    fn find_child_by_name(
        &self,
        rsrc: &ResourceSection<'_>,
        name: &str,
    ) -> Result<Option<NodeChild>, WevtTemplateExtractError> {
        for entry in &self.entries {
            let entry = *entry;
            if !entry.has_name() {
                continue;
            }
            match entry.identifier(rsrc)? {
                ResourceIdentifier::Name(n) if n == name => return Ok(Some(entry.child(rsrc)?)),
                _ => {}
            };
        }
        Ok(None)
    }

    fn children(
        &self,
        rsrc: &ResourceSection<'_>,
    ) -> Result<Vec<(ResourceNodeEntry, NodeChild)>, WevtTemplateExtractError> {
        let mut out = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let entry = *entry;
            out.push((entry, entry.child(rsrc)?));
        }
        Ok(out)
    }
}

enum NodeChild {
    Node(ResourceNode),
    Data(ResourceDataDescriptor),
}

struct ResourceDataDescriptor {
    rva: u32,
    size: u32,
}

impl ResourceDataDescriptor {
    fn read(rsrc: &ResourceSection<'_>, offset: usize) -> Result<Self, WevtTemplateExtractError> {
        Ok(ResourceDataDescriptor {
            rva: rsrc.read_u32(offset)?,
            size: rsrc.read_u32(offset + 4)?,
        })
    }

    fn data(&self, pe: &PeView<'_>) -> Result<Vec<u8>, WevtTemplateExtractError> {
        Ok(pe.read_rva(self.rva, self.size as usize)?.to_vec())
    }
}

/// Extract `WEVT_TEMPLATE` resource blobs from a PE file.
///
/// Returns an empty vector if the PE has no resources or no `WEVT_TEMPLATE` resources.
pub fn extract_wevt_template_resources(
    pe_bytes: &[u8],
) -> Result<Vec<WevtTemplateResource>, WevtTemplateExtractError> {
    let Some(pe) = PeView::parse(pe_bytes)? else {
        return Ok(Vec::new());
    };

    let rsrc = ResourceSection::from_pe(&pe)?;
    let root = ResourceNode::read(&rsrc, 0)?;

    let Some(NodeChild::Node(wevt_root)) = root.find_child_by_name(&rsrc, "WEVT_TEMPLATE")? else {
        return Ok(Vec::new());
    };

    let mut out = vec![];

    // Resource tree layout:
    //   root / "WEVT_TEMPLATE" / <resource-id> / <lang-id> -> data
    for (resource_entry, resource_child) in wevt_root.children(&rsrc)? {
        let NodeChild::Node(resource_node) = resource_child else {
            continue;
        };

        let resource_id = resource_entry.identifier(&rsrc)?;

        for (lang_entry, lang_child) in resource_node.children(&rsrc)? {
            let ResourceIdentifier::Id(lang_id) = lang_entry.identifier(&rsrc)? else {
                continue;
            };

            let NodeChild::Data(descriptor) = lang_child else {
                continue;
            };

            out.push(WevtTemplateResource {
                resource: resource_id.clone(),
                lang_id,
                data: descriptor.data(&pe)?,
            });
        }
    }

    Ok(out)
}

/// Research-only parser for `TTBL`/`TEMP` structures within a `WEVT_TEMPLATE` resource blob.
///
/// Many real-world blobs contain multiple `TTBL` sections. This function finds all parseable
/// `TTBL` sections and returns references to all `TEMP` entries contained within them.
///
/// This uses the CRIM/WEVT provider element directory to locate `TTBL` elements, and then parses
/// the `TTBL`/`TEMP` structures.
pub fn extract_temp_templates_from_wevt_blob(
    blob: &[u8],
) -> Result<Vec<WevtTempTemplateRef>, crate::wevt_templates::manifest::WevtManifestError> {
    let mut out = Vec::new();

    let manifest = crate::wevt_templates::manifest::CrimManifest::parse(blob)?;

    for provider in &manifest.providers {
        let Some(ttbl) = provider.wevt.elements.templates.as_ref() else {
            continue;
        };
        for tpl in &ttbl.templates {
            out.push(WevtTempTemplateRef {
                ttbl_offset: ttbl.offset,
                temp_offset: tpl.offset,
                temp_size: tpl.size,
                header: WevtTempTemplateHeader {
                    item_descriptor_count: tpl.item_descriptor_count,
                    item_name_count: tpl.item_name_count,
                    template_items_offset: tpl.template_items_offset,
                    event_type: tpl.event_type,
                    guid: tpl.guid.clone(),
                },
            });
        }
    }

    Ok(out)
}

const TEMP_BINXML_OFFSET: usize = 40;

/// Parse the BinXML fragment embedded inside a `TEMP` entry.
///
/// Returns `(tokens, bytes_consumed)` where `bytes_consumed` is the number of bytes read from the
/// BinXML fragment (starting at offset 40 from the beginning of `TEMP`).
pub fn parse_temp_binxml_fragment<'a>(
    temp_bytes: &'a [u8],
    ansi_codec: EncodingRef,
) -> crate::err::Result<(
    Vec<crate::model::deserialized::BinXMLDeserializedTokens<'a>>,
    u32,
)> {
    use crate::binxml::deserializer::BinXmlDeserializer;
    use crate::binxml::name::BinXmlNameEncoding;
    use crate::err::EvtxError;

    if temp_bytes.len() < TEMP_BINXML_OFFSET {
        return Err(EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        )));
    }

    let binxml = &temp_bytes[TEMP_BINXML_OFFSET..];
    let de = BinXmlDeserializer::init_with_name_encoding(
        binxml,
        0,
        None,
        false,
        ansi_codec,
        BinXmlNameEncoding::WevtInline,
    );

    let mut iterator = de.iter_tokens(None)?;
    let mut tokens = vec![];
    for t in iterator.by_ref() {
        tokens.push(t?);
    }

    let bytes_consumed = u32::try_from(iterator.position())
        .map_err(|_| EvtxError::calculation_error("BinXML fragment too large".to_string()))?;

    Ok((tokens, bytes_consumed))
}

/// Parse a WEVT_TEMPLATE BinXML fragment (inline-name encoding).
///
/// Returns `(tokens, bytes_consumed)` where `bytes_consumed` is the number of bytes read from `binxml`.
pub fn parse_wevt_binxml_fragment<'a>(
    binxml: &'a [u8],
    ansi_codec: EncodingRef,
) -> crate::err::Result<(
    Vec<crate::model::deserialized::BinXMLDeserializedTokens<'a>>,
    u32,
)> {
    use crate::binxml::deserializer::BinXmlDeserializer;
    use crate::binxml::name::BinXmlNameEncoding;
    use crate::err::EvtxError;

    let de = BinXmlDeserializer::init_with_name_encoding(
        binxml,
        0,
        None,
        false,
        ansi_codec,
        BinXmlNameEncoding::WevtInline,
    );

    let mut iterator = de.iter_tokens(None)?;
    let mut tokens = vec![];
    for t in iterator.by_ref() {
        tokens.push(t?);
    }

    let bytes_consumed = u32::try_from(iterator.position())
        .map_err(|_| EvtxError::calculation_error("BinXML fragment too large".to_string()))?;

    Ok((tokens, bytes_consumed))
}

/// Render a `TEMP` entry to an XML string (with `{sub:N}` placeholders for substitutions).
pub fn render_temp_to_xml(
    temp_bytes: &[u8],
    ansi_codec: EncodingRef,
) -> crate::err::Result<String> {
    use crate::ParserSettings;
    use crate::binxml::name::read_wevt_inline_name_at;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::err::{EvtxError, Result};
    use crate::model::xml::{XmlElement, XmlElementBuilder, XmlModel, XmlPIBuilder};
    use crate::xml_output::{BinXmlOutput, XmlOutput};
    use std::borrow::Cow;

    if temp_bytes.len() < TEMP_BINXML_OFFSET {
        return Err(EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        )));
    }

    let binxml = &temp_bytes[TEMP_BINXML_OFFSET..];
    let (tokens, _bytes_consumed) = parse_temp_binxml_fragment(temp_bytes, ansi_codec)?;

    fn resolve_name<'a>(
        binxml: &'a [u8],
        name_ref: &crate::binxml::name::BinXmlNameRef,
    ) -> Result<Cow<'a, crate::binxml::name::BinXmlName>> {
        Ok(Cow::Owned(read_wevt_inline_name_at(
            binxml,
            name_ref.offset,
        )?))
    }

    // Build a record model similar to `binxml::assemble::create_record_model`,
    // but resolving names via WEVT inline-name decoding and allowing substitution placeholders.
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            crate::model::deserialized::BinXMLDeserializedTokens::FragmentHeader(_) => {}
            crate::model::deserialized::BinXMLDeserializedTokens::TemplateInstance(_) => {
                return Err(EvtxError::Unimplemented {
                    name: "TemplateInstance inside WEVT TEMP BinXML".to_string(),
                });
            }
            crate::model::deserialized::BinXMLDeserializedTokens::AttributeList => {}
            crate::model::deserialized::BinXMLDeserializedTokens::CloseElement => {
                model.push(XmlModel::CloseElement);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseStartElement => {
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close start - Bad parser state",
                        ));
                    }
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish()?)),
                };
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CDATASection => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CharRef => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EntityRef(ref entity) => {
                model.push(XmlModel::EntityRef(resolve_name(binxml, &entity.name)?))
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PITarget(ref name) => {
                let mut builder = XmlPIBuilder::new();
                builder.name(resolve_name(binxml, &name.name)?);
                current_pi = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PIData(data) => {
                match current_pi.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "PI Data without PI target - Bad parser state",
                        ));
                    }
                    Some(mut builder) => {
                        builder.data(Cow::Owned(data));
                        model.push(builder.finish());
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Substitution(sub) => {
                let placeholder = format!("{{sub:{}}}", sub.substitution_index);
                let value = BinXmlValue::StringType(placeholder);
                match current_element {
                    None => model.push(XmlModel::Value(Cow::Owned(value))),
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EndOfStream => {
                model.push(XmlModel::EndOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::StartOfStream => {
                model.push(XmlModel::StartOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseEmptyElement => {
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close empty - Bad parser state",
                        ));
                    }
                    Some(builder) => {
                        model.push(XmlModel::OpenElement(builder.finish()?));
                        model.push(XmlModel::CloseElement);
                    }
                };
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Attribute(ref attr) => {
                if current_element.is_none() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "attribute - Bad parser state",
                    ));
                }
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_name(resolve_name(binxml, &attr.name)?)
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::OpenStartElement(ref elem) => {
                let mut builder = XmlElementBuilder::new();
                builder.name(resolve_name(binxml, &elem.name)?);
                current_element = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Value(value) => {
                match current_element {
                    None => match value {
                        BinXmlValue::EvtXml => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "Unexpected EvtXml in WEVT TEMP BinXML",
                            ));
                        }
                        _ => {
                            model.push(XmlModel::Value(Cow::Owned(value)));
                        }
                    },
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
        }
    }

    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut output = XmlOutput::with_writer(Vec::new(), &settings);

    output.visit_start_of_stream()?;
    let mut stack: Vec<XmlElement> = Vec::new();

    for owned_token in model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                stack.push(open_element);
                output.visit_open_start_element(stack.last().ok_or({
                    EvtxError::FailedToCreateRecordModel(
                        "Invalid parser state - expected stack to be non-empty",
                    )
                })?)?;
            }
            XmlModel::CloseElement => {
                let close_element = stack.pop().ok_or({
                    EvtxError::FailedToCreateRecordModel(
                        "Invalid parser state - expected stack to be non-empty",
                    )
                })?;
                output.visit_close_element(&close_element)?
            }
            XmlModel::Value(s) => output.visit_characters(s)?,
            XmlModel::EndOfStream => {}
            XmlModel::StartOfStream => {}
            XmlModel::PI(pi) => output.visit_processing_instruction(&pi)?,
            XmlModel::EntityRef(entity) => output.visit_entity_reference(&entity)?,
        };
    }

    output.visit_end_of_stream()?;

    String::from_utf8(output.into_writer()).map_err(|e| EvtxError::calculation_error(e.to_string()))
}

/// Render a parsed template definition to XML.
///
/// Compared to `render_temp_to_xml`, this variant can annotate substitutions using the parsed
/// template item descriptors/names (from the CRIM blob).
pub fn render_template_definition_to_xml(
    template: &crate::wevt_templates::manifest::TemplateDefinition<'_>,
    ansi_codec: EncodingRef,
) -> crate::err::Result<String> {
    use crate::ParserSettings;
    use crate::binxml::name::read_wevt_inline_name_at;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::err::{EvtxError, Result};
    use crate::model::xml::{XmlElement, XmlElementBuilder, XmlModel, XmlPIBuilder};
    use crate::xml_output::{BinXmlOutput, XmlOutput};
    use std::borrow::Cow;

    let binxml = template.binxml;
    let (tokens, _bytes_consumed) = parse_wevt_binxml_fragment(binxml, ansi_codec)?;

    fn resolve_name<'a>(
        binxml: &'a [u8],
        name_ref: &crate::binxml::name::BinXmlNameRef,
    ) -> Result<Cow<'a, crate::binxml::name::BinXmlName>> {
        Ok(Cow::Owned(read_wevt_inline_name_at(
            binxml,
            name_ref.offset,
        )?))
    }

    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            crate::model::deserialized::BinXMLDeserializedTokens::FragmentHeader(_) => {}
            crate::model::deserialized::BinXMLDeserializedTokens::TemplateInstance(_) => {
                return Err(EvtxError::Unimplemented {
                    name: "TemplateInstance inside WEVT TEMP BinXML".to_string(),
                });
            }
            crate::model::deserialized::BinXMLDeserializedTokens::AttributeList => {}
            crate::model::deserialized::BinXMLDeserializedTokens::CloseElement => {
                model.push(XmlModel::CloseElement);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseStartElement => {
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close start - Bad parser state",
                        ));
                    }
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish()?)),
                };
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CDATASection => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CharRef => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EntityRef(ref entity) => {
                model.push(XmlModel::EntityRef(resolve_name(binxml, &entity.name)?))
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PITarget(ref name) => {
                let mut builder = XmlPIBuilder::new();
                builder.name(resolve_name(binxml, &name.name)?);
                current_pi = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PIData(data) => {
                match current_pi.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "PI Data without PI target - Bad parser state",
                        ));
                    }
                    Some(mut builder) => {
                        builder.data(Cow::Owned(data));
                        model.push(builder.finish());
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Substitution(sub) => {
                let idx = sub.substitution_index as usize;
                let mut placeholder = format!("{{sub:{idx}}}");

                if let Some(name) = template
                    .items
                    .get(idx)
                    .and_then(|item| item.name.as_deref())
                {
                    placeholder = format!("{{sub:{idx}:{name}}}");
                }

                let value = BinXmlValue::StringType(placeholder);
                match current_element {
                    None => model.push(XmlModel::Value(Cow::Owned(value))),
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EndOfStream => {
                model.push(XmlModel::EndOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::StartOfStream => {
                model.push(XmlModel::StartOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseEmptyElement => {
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close empty - Bad parser state",
                        ));
                    }
                    Some(builder) => {
                        model.push(XmlModel::OpenElement(builder.finish()?));
                        model.push(XmlModel::CloseElement);
                    }
                };
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Attribute(ref attr) => {
                if current_element.is_none() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "attribute - Bad parser state",
                    ));
                }
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_name(resolve_name(binxml, &attr.name)?)
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::OpenStartElement(ref elem) => {
                let mut builder = XmlElementBuilder::new();
                builder.name(resolve_name(binxml, &elem.name)?);
                current_element = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Value(value) => {
                match current_element {
                    None => match value {
                        BinXmlValue::EvtXml => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "Unexpected EvtXml in WEVT TEMP BinXML",
                            ));
                        }
                        _ => {
                            model.push(XmlModel::Value(Cow::Owned(value)));
                        }
                    },
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
        }
    }

    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut output = XmlOutput::with_writer(Vec::new(), &settings);

    output.visit_start_of_stream()?;
    let mut stack: Vec<XmlElement> = Vec::new();

    for owned_token in model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                stack.push(open_element);
                output.visit_open_start_element(stack.last().ok_or({
                    EvtxError::FailedToCreateRecordModel(
                        "Invalid parser state - expected stack to be non-empty",
                    )
                })?)?;
            }
            XmlModel::CloseElement => {
                let close_element = stack.pop().ok_or({
                    EvtxError::FailedToCreateRecordModel(
                        "Invalid parser state - expected stack to be non-empty",
                    )
                })?;
                output.visit_close_element(&close_element)?
            }
            XmlModel::Value(s) => output.visit_characters(s)?,
            XmlModel::EndOfStream => {}
            XmlModel::StartOfStream => {}
            XmlModel::PI(pi) => output.visit_processing_instruction(&pi)?,
            XmlModel::EntityRef(entity) => output.visit_entity_reference(&entity)?,
        };
    }

    output.visit_end_of_stream()?;

    String::from_utf8(output.into_writer()).map_err(|e| EvtxError::calculation_error(e.to_string()))
}

fn read_u16(buf: &[u8], offset: usize) -> Option<u16> {
    let bytes: [u8; 2] = buf.get(offset..offset + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn read_u32(buf: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = buf.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}
