use std::collections::HashMap;
use winstructs::guid::Guid;

#[derive(Debug, Clone)]
pub struct CrimManifest<'a> {
    /// Slice limited to CRIM.size (no trailing bytes).
    pub data: &'a [u8],
    pub header: CrimHeader,
    pub providers: Vec<Provider<'a>>,
}

#[derive(Debug, Clone)]
pub struct CrimHeader {
    pub size: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub provider_count: u32,
}

#[derive(Debug, Clone)]
pub struct Provider<'a> {
    pub guid: Guid,
    /// Offset of the WEVT provider data, relative to the start of the CRIM blob.
    pub offset: u32,
    pub wevt: WevtProvider<'a>,
}

#[derive(Debug, Clone)]
pub struct WevtProvider<'a> {
    pub offset: u32,
    pub size: u32,
    pub message_identifier: Option<u32>,
    pub element_descriptors: Vec<ProviderElementDescriptor>,
    pub unknown2: Vec<u32>,
    pub elements: ProviderElements<'a>,
}

#[derive(Debug, Clone)]
pub struct ProviderElementDescriptor {
    /// Offset of the element (e.g. CHAN/EVNT/TTBL), relative to the start of the CRIM blob.
    pub element_offset: u32,
    pub unknown: u32,
    pub signature: [u8; 4],
}

#[derive(Debug, Clone, Default)]
pub struct ProviderElements<'a> {
    pub channels: Option<ChannelDefinitions>,
    pub events: Option<EventDefinitions>,
    pub keywords: Option<KeywordDefinitions>,
    pub levels: Option<LevelDefinitions>,
    pub maps: Option<MapsDefinitions<'a>>,
    pub opcodes: Option<OpcodeDefinitions>,
    pub tasks: Option<TaskDefinitions>,
    pub templates: Option<TemplateTable<'a>>,
    pub unknown: Vec<UnknownElement<'a>>,
}

#[derive(Debug, Clone)]
pub struct UnknownElement<'a> {
    pub signature: [u8; 4],
    pub offset: u32,
    pub size: u32,
    pub data: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct ChannelDefinitions {
    pub offset: u32,
    pub size: u32,
    pub channels: Vec<ChannelDefinition>,
}

#[derive(Debug, Clone)]
pub struct ChannelDefinition {
    pub identifier: u32,
    pub name_offset: u32,
    pub unknown: u32,
    pub message_identifier: Option<u32>,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EventDefinitions {
    pub offset: u32,
    pub size: u32,
    pub unknown: u32,
    pub events: Vec<EventDefinition>,
    /// Trailing bytes within the EVNT element (currently undocumented).
    pub trailing: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct EventDefinition {
    pub identifier: u16,
    pub version: u8,
    pub channel: u8,
    pub level: u8,
    pub opcode: u8,
    pub task: u16,
    pub keywords: u64,
    pub message_identifier: u32,
    pub template_offset: Option<u32>,
    pub opcode_offset: Option<u32>,
    pub level_offset: Option<u32>,
    pub task_offset: Option<u32>,
    pub unknown_count: u32,
    pub unknown_offset: u32,
    pub flags: u32,
}

/// A stable key for joining provider event metadata to a template definition.
///
/// This mirrors the fields in the `EVNT` event definition header and is intended to be used
/// alongside `template_offset` → `TEMP` resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventKey {
    pub provider_guid: String,
    pub event_id: u16,
    pub version: u8,
    pub channel: u8,
    pub level: u8,
    pub opcode: u8,
    pub task: u16,
    pub keywords: u64,
}

#[derive(Debug)]
pub struct CrimManifestIndex<'a> {
    /// Template GUID → one or more template definitions (duplicates are unexpected but handled).
    pub templates_by_guid: HashMap<String, Vec<&'a TemplateDefinition<'a>>>,
    /// EventKey → one or more template GUIDs (event definitions can legitimately share templates).
    pub event_to_template_guids: HashMap<EventKey, Vec<Guid>>,
}

#[derive(Debug, Clone)]
pub struct KeywordDefinitions {
    pub offset: u32,
    pub size: u32,
    pub keywords: Vec<KeywordDefinition>,
}

#[derive(Debug, Clone)]
pub struct KeywordDefinition {
    pub identifier: u64,
    pub message_identifier: Option<u32>,
    pub data_offset: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LevelDefinitions {
    pub offset: u32,
    pub size: u32,
    pub levels: Vec<LevelDefinition>,
}

#[derive(Debug, Clone)]
pub struct LevelDefinition {
    pub identifier: u32,
    pub message_identifier: Option<u32>,
    pub data_offset: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OpcodeDefinitions {
    pub offset: u32,
    pub size: u32,
    pub opcodes: Vec<OpcodeDefinition>,
}

#[derive(Debug, Clone)]
pub struct OpcodeDefinition {
    pub identifier: u32,
    pub message_identifier: Option<u32>,
    pub data_offset: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TaskDefinitions {
    pub offset: u32,
    pub size: u32,
    pub tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Clone)]
pub struct TaskDefinition {
    pub identifier: u32,
    pub message_identifier: Option<u32>,
    pub mui_identifier: Guid,
    pub data_offset: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TemplateTable<'a> {
    pub offset: u32,
    pub size: u32,
    pub templates: Vec<TemplateDefinition<'a>>,
}

#[derive(Debug, Clone)]
pub struct TemplateDefinition<'a> {
    pub offset: u32,
    pub size: u32,
    pub item_descriptor_count: u32,
    pub item_name_count: u32,
    pub template_items_offset: u32,
    pub event_type: u32,
    pub guid: Guid,
    pub binxml: &'a [u8],
    pub items: Vec<TemplateItem>,
}

#[derive(Debug, Clone)]
pub struct TemplateItem {
    pub unknown1: u32,
    pub input_type: u8,
    pub output_type: u8,
    pub unknown3: u16,
    pub unknown4: u32,
    pub count: u16,
    pub length: u16,
    pub name_offset: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MapsDefinitions<'a> {
    pub offset: u32,
    pub size: u32,
    pub maps: Vec<MapDefinition<'a>>,
}

#[derive(Debug, Clone)]
pub enum MapDefinition<'a> {
    ValueMap(ValueMap<'a>),
    Bitmap(BitmapMap<'a>),
    Unknown {
        signature: [u8; 4],
        offset: u32,
        data: &'a [u8],
    },
}

#[derive(Debug, Clone)]
pub struct ValueMap<'a> {
    pub offset: u32,
    pub size: u32,
    pub map_string_offset: u32,
    pub entries: Vec<ValueMapEntry>,
    pub map_string: Option<String>,
    /// Trailing bytes within this VMAP (if any).
    pub trailing: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct ValueMapEntry {
    pub identifier: u32,
    pub message_identifier: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct BitmapMap<'a> {
    pub offset: u32,
    pub data: &'a [u8],
}

impl Provider<'_> {
    /// Resolve a template definition by its offset (as stored in EVNT.template_offset).
    pub fn template_by_offset(&self, offset: u32) -> Option<&TemplateDefinition<'_>> {
        self.wevt
            .elements
            .templates
            .as_ref()
            .and_then(|t| t.templates.iter().find(|tpl| tpl.offset == offset))
    }
}


