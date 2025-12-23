use winstructs::guid::Guid;

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


