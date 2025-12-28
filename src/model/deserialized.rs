use crate::binxml::name::BinXmlNameRef;
use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType};

use crate::ChunkOffset;
use crate::EvtxChunk;
use crate::utils::ByteCursor;
use std::fmt::{self, Formatter};
use winstructs::guid::Guid;

#[derive(Debug, PartialEq, Clone)]
pub enum BinXMLDeserializedTokens<'a> {
    FragmentHeader(BinXMLFragmentHeader),
    TemplateInstance(BinXmlTemplateRef),
    OpenStartElement(BinXMLOpenStartElement),
    AttributeList,
    Attribute(BinXMLAttribute),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value(BinXmlValue<'a>),
    CDATASection,
    CharRef,
    EntityRef(BinXmlEntityReference),
    PITarget(BinXMLProcessingInstructionTarget),
    PIData(String),
    Substitution(TemplateSubstitutionDescriptor),
    EndOfStream,
    StartOfStream,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLProcessingInstructionTarget {
    pub name: BinXmlNameRef,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLOpenStartElement {
    pub data_size: u32,
    pub name: BinXmlNameRef,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlTemplateDefinitionHeader {
    /// A pointer to the next template in the bucket.
    pub next_template_offset: ChunkOffset,
    pub guid: Guid,
    pub data_size: u32,
}

impl fmt::Display for BinXmlTemplateDefinitionHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<BinXmlTemplateDefinitionHeader - id: {guid}, data_size: {size}>",
            guid = self.guid,
            size = self.data_size
        )
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct BinXMLTemplateDefinition<'a> {
    pub header: BinXmlTemplateDefinitionHeader,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXmlEntityReference {
    pub name: BinXmlNameRef,
}

#[derive(Debug, PartialEq, Clone)]
pub struct BinXmlTemplateRef {
    pub template_id: u32,
    pub template_def_offset: ChunkOffset,
    /// When the template definition header is embedded inline in the record's TemplateInstance,
    /// we can read the template GUID directly. Otherwise, the GUID lives in the template
    /// definition referenced by `template_def_offset` (typically in the chunk template table).
    pub template_guid: Option<Guid>,
    pub substitutions: Vec<TemplateSubstitutionSpan>,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct TemplateValueDescriptor {
    pub size: u16,
    pub value_type: BinXmlValueType,
}

/// A raw substitution value span within a record's `TemplateInstance`.
///
/// This stores the **location** and **declared type/size** of the substitution value, allowing
/// callers to decode only when needed (e.g. during JSON/XML streaming expansion).
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct TemplateSubstitutionSpan {
    pub offset: ChunkOffset,
    pub size: u16,
    pub value_type: BinXmlValueType,
}

impl TemplateSubstitutionSpan {
    /// Decode this substitution value from the owning chunk buffer.
    ///
    /// Note: decoding may allocate into the chunk arena (for strings / nested BinXML).
    pub fn decode<'a>(
        &self,
        chunk: &'a EvtxChunk<'a>,
    ) -> std::result::Result<BinXmlValue<'a>, crate::err::DeserializationError> {
        let mut cursor = ByteCursor::with_pos(chunk.data, self.offset as usize)?;
        BinXmlValue::deserialize_value_type_cursor(
            &self.value_type,
            &mut cursor,
            Some(chunk),
            chunk.arena,
            Some(self.size),
            chunk.settings.get_ansi_codec(),
        )
    }
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct TemplateSubstitutionDescriptor {
    // Zero-based (0 is first replacement)
    pub substitution_index: u16,
    pub value_type: BinXmlValueType,
    pub ignore: bool,
}

#[repr(C)]
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLFragmentHeader {
    pub major_version: u8,
    pub minor_version: u8,
    pub flags: u8,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLAttribute {
    pub name: BinXmlNameRef,
}
