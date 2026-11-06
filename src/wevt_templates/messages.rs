//! Event messages from provider binaries: `RT_MESSAGETABLE` strings joined with `WEVT_TEMPLATE`
//! manifests.
//!
//! A provider's manifest (the `WEVT_TEMPLATE` resource of its resource file) gives each event a
//! message id, and names its levels, tasks, opcodes and keywords by message id. The strings are in
//! the `RT_MESSAGETABLE` resource of the provider's message file, or, on Windows Vista and later,
//! of its `.mui` file (for example `en-US\adtschema.dll.mui`). A message has inserts such as `%1`,
//! which Windows fills with the event's data, in order.
//!
//! [`MessageCatalog`] collects these from binaries copied off a Windows system. With
//! [`ParserSettings::messages`](crate::ParserSettings::messages) set, every record whose provider
//! is in the catalog gets a `RenderingInfo` element with its message, level, task, opcode and
//! keyword names, like the one an `.MTA` file gives (see [`crate::mta`]). An `.MTA` file's strings
//! come first: a record that already has a `RenderingInfo` keeps it.
//!
//! The catalog renders messages the way `FormatMessage` does, with two differences: an insert's
//! printf-style format (`%1!x!`) is ignored, so the value renders as in the event's XML, and
//! an insert with no value stays as written.
//!
//! ```no_run
//! use std::sync::Arc;
//! use evtx::{EvtxParser, ParserSettings, wevt_templates::messages::MessageCatalog};
//!
//! // A copy of C:\Windows\System32, with its en-US folder.
//! let catalog = MessageCatalog::from_directory("System32", Some("en-US"))?;
//! let settings = ParserSettings::new().messages(Some(Arc::new(catalog)));
//! let mut parser = EvtxParser::from_path("Security.evtx")?.with_configuration(settings);
//! for record in parser.records_json_value() {
//!     let record = record?;
//!     println!("{}", record.data["Event"]["RenderingInfo"]["Message"]);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bumpalo::Bump;
use thiserror::Error;

use super::WevtTemplateExtractError;
use super::extract::{RT_MESSAGETABLE, extract_resources, extract_wevt_template_resources};
use super::manifest::{CrimManifest, EventDefinition, WevtManifestError};
use super::normalize_guid;
use super::types::ResourceIdentifier;
use crate::binxml::value_variant::BinXmlValue;
use crate::model::ir::{Element, IrTree, Node, Text};
use crate::mta::RenderingInfo;
use crate::mta::{decode_utf16, narrow, number_value};

/// The language id of `en-US`, preferred when a binary has message tables in several languages.
const EN_US: u32 = 1033;

/// An error while building a [`MessageCatalog`].
#[derive(Debug, Error)]
pub enum MessageCatalogError {
    #[error("failed to read `{}`", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read the resources of a binary: {0}")]
    Pe(#[from] WevtTemplateExtractError),
    #[error("failed to parse a WEVT_TEMPLATE manifest: {0}")]
    Manifest(#[from] WevtManifestError),
    #[error("malformed message table: {0}")]
    MessageTable(&'static str),
}

/// The strings of a binary's `RT_MESSAGETABLE`, by message id.
#[derive(Debug, Clone, Default)]
pub struct MessageTable {
    messages: HashMap<u32, Box<str>>,
}

impl MessageTable {
    /// Parses a `MESSAGE_RESOURCE_DATA` structure: the content of one `RT_MESSAGETABLE`
    /// resource.
    pub fn parse(data: &[u8]) -> Result<Self, MessageCatalogError> {
        let u32_at = |at: usize| {
            data.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let malformed = MessageCatalogError::MessageTable;
        let blocks = u32_at(0).ok_or(malformed("no block count"))? as usize;
        let mut messages = HashMap::new();
        for block in 0..blocks.min(data.len() / 12) {
            let at = 4 + block * 12;
            let (Some(low), Some(high), Some(mut entry)) =
                (u32_at(at), u32_at(at + 4), u32_at(at + 8))
            else {
                return Err(malformed("block runs past the table"));
            };
            for id in low..=high {
                let entry_at = entry as usize;
                let header = data
                    .get(entry_at..entry_at + 4)
                    .ok_or(malformed("entry runs past the table"))?;
                let length = usize::from(u16::from_le_bytes([header[0], header[1]]));
                let flags = u16::from_le_bytes([header[2], header[3]]);
                let text = data
                    .get(entry_at + 4..entry_at + length.max(4))
                    .ok_or(malformed("entry text runs past the table"))?;
                if length < 4 {
                    return Err(malformed("entry shorter than its header"));
                }
                let text = if flags & 1 != 0 {
                    String::from(decode_utf16(text))
                } else {
                    // ANSI entries: Latin-1 is close enough for the ASCII Windows writes there.
                    text.iter().map(|&b| char::from(b)).collect()
                };
                let text = text.trim_end_matches('\0');
                messages.insert(id, text.into());
                entry = entry.wrapping_add(length as u32);
                if id == u32::MAX {
                    break;
                }
            }
        }
        Ok(MessageTable { messages })
    }

    /// Reads the message table of a PE binary (`.dll`, `.exe` or `.mui`), preferring `en-US`
    /// when it has several languages. Empty when the binary has no message table.
    pub fn from_pe_bytes(pe_bytes: &[u8]) -> Result<Self, MessageCatalogError> {
        let resources = extract_resources(pe_bytes, &ResourceIdentifier::Id(RT_MESSAGETABLE))?;
        let chosen = resources
            .iter()
            .find(|r| r.lang_id == EN_US)
            .or_else(|| resources.first());
        match chosen {
            Some(resource) => Self::parse(&resource.data),
            None => Ok(MessageTable::default()),
        }
    }

    /// The message with id `id`, as stored (with its inserts and the line break it ends with).
    pub fn get(&self, id: u32) -> Option<&str> {
        self.messages.get(&id).map(AsRef::as_ref)
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// The input and output types of an event template's items, in order.
type ItemTypes = Vec<(u8, u8)>;

/// The messages of one provider.
#[derive(Debug, Clone, Default)]
struct ProviderMessages {
    /// The tables to look message ids up in: the provider's resource file, then its `.mui` files.
    tables: Vec<usize>,
    /// Event message ids by `(event id, version)`, with the rest of each event's descriptor to
    /// pick between events that share both.
    events: HashMap<(u16, u8), Vec<(EventDefinition, ItemTypes)>>,
    levels: HashMap<u32, u32>,
    tasks: HashMap<u32, u32>,
    /// By `(opcode << 16) | task`, with task 0 for opcodes not tied to a task.
    opcodes: HashMap<u32, u32>,
    /// By keyword mask.
    keywords: HashMap<u64, u32>,
}

/// Event messages from provider binaries. See [the module docs](self).
#[derive(Debug, Clone, Default)]
pub struct MessageCatalog {
    tables: Vec<MessageTable>,
    /// Manifest providers by normalized GUID.
    providers: HashMap<String, ProviderMessages>,
    /// Classic event sources, by ASCII-lowercased name.
    sources: HashMap<String, Vec<usize>>,
    /// Tables that `%%<id>` values in event data are looked up in after the provider's own,
    /// such as `msobjs.dll`'s for the Security log.
    parameter_tables: Vec<usize>,
}

impl MessageCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of manifest providers and classic sources in the catalog.
    pub fn provider_count(&self) -> usize {
        self.providers.len() + self.sources.len()
    }

    fn add_table(&mut self, table: MessageTable) -> Option<usize> {
        if table.is_empty() {
            return None;
        }
        self.tables.push(table);
        Some(self.tables.len() - 1)
    }

    /// Adds the manifest providers of `resource_file` (a binary with a `WEVT_TEMPLATE`), with
    /// strings from its own message table and from `message_files` (its `.mui` files). Returns
    /// how many providers it added; a provider already in the catalog keeps its first strings.
    pub fn add_provider_files(
        &mut self,
        resource_file: &[u8],
        message_files: &[&[u8]],
    ) -> Result<usize, MessageCatalogError> {
        let manifests = extract_wevt_template_resources(resource_file)?;
        if manifests.is_empty() {
            return Ok(0);
        }
        let mut tables = Vec::new();
        for bytes in std::iter::once(&resource_file).chain(message_files) {
            if let Some(index) = self.add_table(MessageTable::from_pe_bytes(bytes)?) {
                tables.push(index);
            }
        }
        let mut added = 0;
        for manifest in &manifests {
            let manifest = CrimManifest::parse(&manifest.data)?;
            for provider in &manifest.providers {
                let guid = normalize_guid(&provider.guid.to_string());
                if self.providers.contains_key(&guid) {
                    continue;
                }
                let elements = &provider.wevt.elements;
                // Each template's item input and output types, by the template's offset.
                let input_types: HashMap<u32, Vec<(u8, u8)>> = elements
                    .templates
                    .iter()
                    .flat_map(|t| &t.templates)
                    .map(|t| {
                        let types = t.items.iter().map(|i| (i.input_type, i.output_type));
                        (t.offset, types.collect())
                    })
                    .collect();
                let mut messages = ProviderMessages {
                    tables: tables.clone(),
                    ..Default::default()
                };
                for event in elements.events.iter().flat_map(|e| &e.events) {
                    messages
                        .events
                        .entry((event.identifier, event.version))
                        .or_default()
                        .push((
                            event.clone(),
                            event
                                .template_offset
                                .and_then(|offset| input_types.get(&offset).cloned())
                                .unwrap_or_default(),
                        ));
                }
                for level in elements.levels.iter().flat_map(|l| &l.levels) {
                    if let Some(id) = level.message_identifier {
                        messages.levels.insert(level.identifier, id);
                    }
                }
                for task in elements.tasks.iter().flat_map(|t| &t.tasks) {
                    if let Some(id) = task.message_identifier {
                        messages.tasks.insert(task.identifier & 0xFFFF, id);
                    }
                }
                for opcode in elements.opcodes.iter().flat_map(|o| &o.opcodes) {
                    if let Some(id) = opcode.message_identifier {
                        // Stored as `(opcode << 16) | task`.
                        messages.opcodes.insert(opcode.identifier, id);
                    }
                }
                for keyword in elements.keywords.iter().flat_map(|k| &k.keywords) {
                    if let Some(id) = keyword.message_identifier {
                        messages.keywords.insert(keyword.identifier, id);
                    }
                }
                self.providers.insert(guid, messages);
                added += 1;
            }
        }
        Ok(added)
    }

    /// Adds a classic event source (an event log provider without a manifest, matched by its
    /// `Provider/@Name`), with strings from its message files. Its events' message ids are
    /// `(Qualifiers << 16) | EventID`.
    pub fn add_classic_source(
        &mut self,
        name: &str,
        message_files: &[&[u8]],
    ) -> Result<(), MessageCatalogError> {
        let mut tables = Vec::new();
        for bytes in message_files {
            if let Some(index) = self.add_table(MessageTable::from_pe_bytes(bytes)?) {
                tables.push(index);
            }
        }
        self.sources
            .entry(name.to_ascii_lowercase())
            .or_default()
            .extend(tables);
        Ok(())
    }

    /// Adds a parameter message file, whose strings replace `%%<id>` in event data when the
    /// provider's own tables don't have the id, as `msobjs.dll` does for the Security log.
    pub fn add_parameter_file(&mut self, pe_bytes: &[u8]) -> Result<(), MessageCatalogError> {
        if let Some(index) = self.add_table(MessageTable::from_pe_bytes(pe_bytes)?) {
            self.parameter_tables.push(index);
        }
        Ok(())
    }

    /// Builds a catalog from a copy of a Windows binaries folder, such as `C:\Windows\System32`.
    /// Every `.dll`, `.exe` and `.sys` under `dir` with a manifest adds its providers, with
    /// strings from the `.mui` file of the same name in the `locale` subfolder of its folder
    /// (`en-US` when `None`). `msobjs.dll` (or its `.mui`) is added as a parameter file.
    ///
    /// Files that aren't PE binaries, or whose resources don't parse, are skipped.
    pub fn from_directory(
        dir: impl AsRef<Path>,
        locale: Option<&str>,
    ) -> Result<Self, MessageCatalogError> {
        let mut catalog = MessageCatalog::new();
        catalog.add_path(dir, locale)?;
        Ok(catalog)
    }

    /// Adds a binaries folder, as [`MessageCatalog::from_directory`] reads one, or a single
    /// binary, with its `.mui` file from the `locale` folder next to it.
    pub fn add_path(
        &mut self,
        path: impl AsRef<Path>,
        locale: Option<&str>,
    ) -> Result<(), MessageCatalogError> {
        let locale = locale.unwrap_or("en-US");
        let path = path.as_ref();
        if path.is_file() {
            return self.add_binary(path, locale, true);
        }
        let mut pending = vec![path.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let entries = std::fs::read_dir(&dir).map_err(|source| MessageCatalogError::Io {
                path: dir.clone(),
                source,
            })?;
            let mut paths: Vec<PathBuf> =
                entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for path in paths {
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                self.add_binary(&path, locale, false)?;
            }
        }
        Ok(())
    }

    /// Adds one binary found in a folder, or given on its own (`strict`), whose errors are then
    /// returned instead of skipped.
    fn add_binary(
        &mut self,
        path: &Path,
        locale: &str,
        strict: bool,
    ) -> Result<(), MessageCatalogError> {
        let read = |path: &Path| {
            std::fs::read(path).map_err(|source| MessageCatalogError::Io {
                path: path.to_path_buf(),
                source,
            })
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return Ok(());
        };
        let lower = name.to_ascii_lowercase();
        if !strict
            && !(lower.ends_with(".dll") || lower.ends_with(".exe") || lower.ends_with(".sys"))
        {
            return Ok(());
        }
        let files = read(path).and_then(|bytes| {
            let mui = path
                .parent()
                .map(|parent| parent.join(locale).join(format!("{name}.mui")))
                .filter(|mui| mui.is_file())
                .map(|mui| read(&mui))
                .transpose()?;
            Ok((bytes, mui))
        });
        let result = files.and_then(|(bytes, mui)| {
            if lower == "msobjs.dll" {
                self.add_parameter_file(mui.as_deref().unwrap_or(&bytes))
            } else {
                let message_files: Vec<&[u8]> = mui.as_deref().into_iter().collect();
                self.add_provider_files(&bytes, &message_files).map(drop)
            }
        });
        match result {
            Err(e) if !strict => {
                log::debug!("skipping `{}`: {e}", path.display());
                Ok(())
            }
            other => other,
        }
    }

    /// Looks up a message id in `tables`.
    fn lookup(&self, tables: &[usize], id: u32) -> Option<&str> {
        tables.iter().find_map(|&t| self.tables[t].get(id))
    }

    /// The strings of an event, or `None` when its provider isn't in the catalog. Allocates the
    /// formatted message in `bump`.
    fn rendering_info<'b>(
        &'b self,
        event: &EventFields<'_>,
        bump: &'b Bump,
    ) -> Option<RenderingInfo<'b>> {
        if let Some(provider) = event
            .provider_guid
            .as_deref()
            .and_then(|guid| self.providers.get(&normalize_guid(guid)))
        {
            let definition = provider
                .events
                .get(&(event.event_id, event.version))
                .and_then(|candidates| {
                    candidates
                        .iter()
                        .find(|(d, _)| {
                            d.level == event.level
                                && d.opcode == event.opcode
                                && d.task == event.task
                                && d.keywords == event.keywords
                        })
                        .or_else(|| candidates.first())
                });
            let message = definition.and_then(|(d, input_types)| {
                let template = self.lookup(&provider.tables, d.message_identifier)?;
                Some(self.format(
                    template,
                    &event.inserts,
                    input_types,
                    &provider.tables,
                    bump,
                ))
            });
            let name = |id: Option<&u32>| {
                id.and_then(|&id| self.lookup(&provider.tables, id))
                    .map(trim_message)
            };
            let level = name(provider.levels.get(&u32::from(event.level)))
                .or_else(|| winmeta_level(event.level));
            let task = name(provider.tasks.get(&u32::from(event.task)))
                .or_else(|| (event.task == 0).then_some("None"));
            let opcode = name(
                provider
                    .opcodes
                    .get(&((u32::from(event.opcode) << 16) | u32::from(event.task)))
                    .or_else(|| provider.opcodes.get(&(u32::from(event.opcode) << 16))),
            )
            .or_else(|| winmeta_opcode(event.opcode));
            let keywords = (0..64)
                .filter(|bit| event.keywords & (1 << bit) != 0)
                .filter_map(|bit| {
                    name(provider.keywords.get(&(1u64 << bit))).or_else(|| winmeta_keyword(bit))
                })
                .collect();
            return Some(RenderingInfo {
                message,
                level,
                task,
                opcode,
                keywords,
            });
        }

        let tables = self
            .sources
            .get(&event.provider_name.to_ascii_lowercase())?;
        let id = (u32::from(event.qualifiers) << 16) | u32::from(event.event_id);
        let message = self
            .lookup(tables, id)
            .map(|template| self.format(template, &event.inserts, &[], tables, bump));
        Some(RenderingInfo {
            message,
            level: winmeta_level(event.level),
            task: None,
            opcode: None,
            keywords: (0..64)
                .filter(|bit| event.keywords & (1 << bit) != 0)
                .filter_map(winmeta_keyword)
                .collect(),
        })
    }

    /// Fills the inserts of `template` with `inserts`, as `FormatMessage` does, and replaces the
    /// `%%<id>` parameters in them.
    fn format<'b>(
        &self,
        template: &str,
        inserts: &[Insert],
        item_types: &[(u8, u8)],
        tables: &[usize],
        bump: &'b Bump,
    ) -> &'b str {
        let mut out = String::with_capacity(template.len());
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some(d @ '1'..='9') => {
                    let mut number = d.to_digit(10).expect("digit");
                    let mut written = format!("%{d}");
                    while let Some(&next) = chars.peek() {
                        match next.to_digit(10) {
                            Some(digit) if number < 10 => {
                                number = number * 10 + digit;
                                written.push(next);
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                    // A printf-style format, such as `!x!`.
                    if chars.peek() == Some(&'!') {
                        let mut spec = String::from("!");
                        chars.next();
                        for c in chars.by_ref() {
                            spec.push(c);
                            if c == '!' {
                                break;
                            }
                        }
                        written.push_str(&spec);
                    }
                    let index = number as usize - 1;
                    match inserts.get(index) {
                        // Windows formats a number by its output type, in upper case hex for
                        // the hex types (pointers keep the XML's lower case).
                        Some(Insert {
                            number: Some(number),
                            ..
                        }) if item_types.get(index).is_some_and(|&(input, output)| {
                            input != INTYPE_POINTER && OUTTYPE_HEX.contains(&output)
                        }) =>
                        {
                            out.push_str(&format!("0x{number:X}"))
                        }
                        Some(insert) => out.push_str(&self.parameters(&insert.text, tables)),
                        None => out.push_str(&written),
                    }
                }
                Some('0') => break,
                Some('n') => out.push_str("\r\n"),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('b') => out.push(' '),
                Some(other) => out.push(other),
                None => out.push('%'),
            }
        }
        bump.alloc_str(trim_message(&out))
    }

    /// Replaces each `%%<id>` in an event data value with the parameter string `id`.
    fn parameters<'v>(&self, value: &'v str, tables: &[usize]) -> Cow<'v, str> {
        if !value.contains("%%") {
            return Cow::Borrowed(value);
        }
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(at) = rest.find("%%") {
            out.push_str(&rest[..at]);
            let digits = rest[at + 2..]
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len() - at - 2);
            let id = rest[at + 2..at + 2 + digits].parse::<u32>().ok();
            let parameter = id.and_then(|id| {
                self.lookup(tables, id)
                    .or_else(|| self.lookup(&self.parameter_tables, id))
            });
            match parameter {
                Some(parameter) => out.push_str(trim_message(parameter)),
                None => out.push_str(&rest[at..at + 2 + digits]),
            }
            rest = &rest[at + 2 + digits..];
        }
        out.push_str(rest);
        Cow::Owned(out)
    }
}

/// Drops the line break (and padding) a message table entry ends with.
fn trim_message(message: &str) -> &str {
    message.trim_end_matches(['\r', '\n', ' ', '\0'])
}

/// The standard (`winmeta`) names, as Windows stores them in `.MTA` files' `DEFAULT_PUBLISHER`.
fn winmeta_level(level: u8) -> Option<&'static str> {
    Some(match level {
        0 => "Log Always",
        1 => "Critical",
        2 => "Error",
        3 => "Warning",
        4 => "Information",
        5 => "Verbose",
        _ => return None,
    })
}

fn winmeta_opcode(opcode: u8) -> Option<&'static str> {
    Some(match opcode {
        0 => "Info",
        1 => "Start",
        2 => "Stop",
        3 => "DCStart",
        4 => "DCStop",
        5 => "Extension",
        6 => "Reply",
        7 => "Resume",
        8 => "Suspend",
        9 => "Send",
        240 => "Receive",
        _ => return None,
    })
}

fn winmeta_keyword(bit: u32) -> Option<&'static str> {
    Some(match bit {
        48 => "Response Time",
        50 => "WDI Diag",
        51 => "SQM",
        52 => "Audit Failure",
        53 => "Audit Success",
        54 => "Correlation Hint",
        55 => "Classic",
        _ => return None,
    })
}

/// The manifest input type of pointers (`win:Pointer`).
const INTYPE_POINTER: u8 = 16;
/// The manifest output types `win:HexInt8` to `win:HexInt64`.
const OUTTYPE_HEX: std::ops::RangeInclusive<u8> = 16..=19;

/// An event data value, as its XML shows it.
#[derive(Debug, Default)]
struct Insert {
    text: String,
    /// The value, when it is an unsigned integer.
    number: Option<u64>,
}

/// The fields of a record's tree that select and fill its message.
#[derive(Debug, Default)]
struct EventFields<'t> {
    provider_name: Cow<'t, str>,
    provider_guid: Option<Cow<'t, str>>,
    event_id: u16,
    qualifiers: u16,
    version: u8,
    level: u8,
    task: u16,
    opcode: u8,
    keywords: u64,
    /// The event's data values, in order: the `Data` elements of `EventData`, or the leaf
    /// elements under `UserData`.
    inserts: Vec<Insert>,
}

impl<'t> EventFields<'t> {
    fn of(tree: &'t IrTree<'_>) -> Option<(Self, bool)> {
        let root = tree.root_element();
        if root.name.as_str() != "Event" {
            return None;
        }
        let mut fields = EventFields::default();
        let mut has_rendering_info = false;
        let mut guid_text = None;
        for node in &root.children {
            let Node::Element(id) = node else { continue };
            let element = tree.element(*id);
            match element.name.as_str() {
                "System" => {
                    for child in element_children(tree, element) {
                        let number = || number_value(&child.children);
                        match child.name.as_str() {
                            "Provider" => {
                                for attr in child.attrs.iter() {
                                    match attr.name.as_str() {
                                        "Name" => fields.provider_name = node_text(&attr.value),
                                        "Guid" => guid_text = Some(node_text(&attr.value)),
                                        _ => {}
                                    }
                                }
                            }
                            "EventID" => {
                                fields.event_id = narrow(number());
                                if let Some(q) =
                                    child.attrs.iter().find(|a| a.name.as_str() == "Qualifiers")
                                {
                                    fields.qualifiers = narrow(number_value(&q.value));
                                }
                            }
                            "Version" => fields.version = narrow(number()),
                            "Level" => fields.level = narrow(number()),
                            "Task" => fields.task = narrow(number()),
                            "Opcode" => fields.opcode = narrow(number()),
                            "Keywords" => fields.keywords = number().unwrap_or_default(),
                            _ => {}
                        }
                    }
                }
                "EventData" => {
                    for data in element_children(tree, element) {
                        fields.inserts.push(insert(&data.children));
                    }
                }
                "UserData" => {
                    for child in element_children(tree, element) {
                        collect_leaves(tree, child, &mut fields.inserts);
                    }
                }
                "RenderingInfo" => has_rendering_info = true,
                _ => {}
            }
        }
        fields.provider_guid = guid_text;
        Some((fields, has_rendering_info))
    }
}

fn element_children<'e, 'a>(
    tree: &'e IrTree<'a>,
    element: &'e Element<'a>,
) -> impl Iterator<Item = &'e Element<'a>> {
    element.children.iter().filter_map(move |node| match node {
        Node::Element(id) => Some(tree.element(*id)),
        _ => None,
    })
}

/// Pushes the text of each leaf element under `element`, in document order.
fn collect_leaves(tree: &IrTree<'_>, element: &Element<'_>, out: &mut Vec<Insert>) {
    if !element.has_element_child {
        out.push(insert(&element.children));
        return;
    }
    for child in element_children(tree, element) {
        collect_leaves(tree, child, out);
    }
}

fn insert(nodes: &[Node<'_>]) -> Insert {
    let number = match nodes {
        [Node::Value(value)] => match *value {
            BinXmlValue::UInt8Type(v) => Some(v.into()),
            BinXmlValue::UInt16Type(v) => Some(v.into()),
            BinXmlValue::UInt32Type(v) | BinXmlValue::HexInt32Type(v) => Some(v.into()),
            BinXmlValue::UInt64Type(v) | BinXmlValue::HexInt64Type(v) => Some(v),
            _ => None,
        },
        _ => None,
    };
    let text = node_text(nodes);
    let text = match nodes {
        [Node::Value(BinXmlValue::FileTimeType(_) | BinXmlValue::SysTimeType(_))] => {
            message_time(&text).unwrap_or_else(|| text.into_owned())
        }
        _ => text.into_owned(),
    };
    Insert { text, number }
}

/// A time as Windows' messages show it: each part of the date behind a left-to-right mark, and
/// nine fractional digits, such as `\u{200e}2016\u{200e}-\u{200e}07\u{200e}-\u{200e}08T18:16:31.108103900Z`
/// for the XML's `2016-07-08T18:16:31.1081039Z`.
fn message_time(xml: &str) -> Option<String> {
    let (date, time) = xml.split_once('T')?;
    let mut parts = date.splitn(3, '-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let time = time.strip_suffix('Z')?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    Some(format!(
        "\u{200e}{year}\u{200e}-\u{200e}{month}\u{200e}-\u{200e}{day}T{clock}.{fraction:0<9}Z"
    ))
}

/// The text of `nodes`, with values as the event's XML shows them (unescaped).
fn node_text<'n>(nodes: &[Node<'n>]) -> Cow<'n, str> {
    let text = |node: &Node<'n>| -> Cow<'n, str> {
        match node {
            Node::Text(Text::Utf8(s)) | Node::Value(BinXmlValue::AnsiStringType(s)) => {
                Cow::Borrowed(*s)
            }
            Node::Text(Text::Utf16(s)) | Node::Value(BinXmlValue::StringType(s)) => {
                Cow::Owned(decode_utf16(s.as_bytes()).into())
            }
            Node::Value(value) => Cow::Owned(value.to_string()),
            Node::CharRef(c) => Cow::Owned(char::from_u32(u32::from(*c)).unwrap_or('_').into()),
            _ => Cow::Borrowed(""),
        }
    };
    match nodes {
        [node] => text(node),
        _ => Cow::Owned(nodes.iter().map(text).collect()),
    }
}

/// Adds a `RenderingInfo` element to a record's tree from `catalog`, unless its provider isn't
/// in the catalog or it has one already.
pub(crate) fn add_rendering_info<'a>(
    tree: &mut IrTree<'a>,
    catalog: &'a MessageCatalog,
    bump: &'a Bump,
) {
    let Some((fields, has_rendering_info)) = EventFields::of(tree) else {
        return;
    };
    if has_rendering_info {
        return;
    }
    let Some(info) = catalog.rendering_info(&fields, bump) else {
        return;
    };
    let root = tree.root();
    let arena = tree.arena_mut();
    let rendering_info = crate::mta::rendering_info_element(&info, arena, bump);
    arena
        .get_mut(root)
        .expect("root is in the arena")
        .push_child(Node::Element(rendering_info));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `MESSAGE_RESOURCE_DATA` with one block of Unicode entries from id `low`.
    fn table(low: u32, messages: &[&str]) -> Vec<u8> {
        let mut entries = Vec::new();
        for message in messages {
            let mut text: Vec<u8> = message.encode_utf16().flat_map(u16::to_le_bytes).collect();
            text.extend_from_slice(&[0, 0]);
            while !text.len().is_multiple_of(4) {
                text.push(0);
            }
            entries.extend_from_slice(&((text.len() + 4) as u16).to_le_bytes());
            entries.extend_from_slice(&1u16.to_le_bytes());
            entries.extend_from_slice(&text);
        }
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&low.to_le_bytes());
        data.extend_from_slice(&(low + messages.len() as u32 - 1).to_le_bytes());
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&entries);
        data
    }

    #[test]
    fn parses_message_tables() {
        let parsed = MessageTable::parse(&table(0x10, &["First\r\n", "Second %1\r\n"])).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.get(0x10), Some("First\r\n"));
        assert_eq!(parsed.get(0x11), Some("Second %1\r\n"));
        assert_eq!(parsed.get(0x12), None);

        let mut truncated = table(1, &["Cut"]);
        truncated.truncate(20);
        assert!(MessageTable::parse(&truncated).is_err());
    }

    fn catalog_with(messages: &[&str]) -> MessageCatalog {
        let mut catalog = MessageCatalog::new();
        let table = MessageTable::parse(&table(100, messages)).unwrap();
        catalog.add_table(table);
        catalog
    }

    fn insert(text: &str) -> Insert {
        Insert {
            text: text.to_string(),
            number: None,
        }
    }

    #[test]
    fn formats_like_format_message() {
        let catalog = MessageCatalog::new();
        let bump = Bump::new();
        let inserts = [insert("alice"), insert("C:\\x"), insert("3")];
        let format = |template| catalog.format(template, &inserts, &[], &[], &bump);
        assert_eq!(
            format("User:%t%1%nPath: %2!s!.%n%n"),
            "User:\talice\r\nPath: C:\\x."
        );
        assert_eq!(format("100%% sure%b%3%0 cut"), "100% sure 3");
        // An insert without a value stays as written, so does an unknown escape.
        assert_eq!(format("%1 and %12!d! %q"), "alice and %12!d! q");
    }

    #[test]
    fn formats_hex_numbers_by_output_type() {
        let catalog = MessageCatalog::new();
        let bump = Bump::new();
        let inserts = [
            Insert {
                text: "999".to_string(),
                number: Some(999),
            },
            Insert {
                text: "0x1c8".to_string(),
                number: Some(0x1c8),
            },
        ];
        // `win:UInt64` shown as `win:HexInt64`, and a `win:Pointer` shown as `win:HexInt64`.
        let types = [(10, 19), (INTYPE_POINTER, 19)];
        assert_eq!(
            catalog.format("%1 %2", &inserts, &types, &[], &bump),
            "0x3E7 0x1c8"
        );
    }

    #[test]
    fn replaces_parameters() {
        let mut catalog = catalog_with(&["Yes\r\n", "No\r\n"]);
        catalog.parameter_tables.push(0);
        assert_eq!(
            catalog.parameters("%%100 then %%101, %%7 and 100%", &[]),
            "Yes then No, %%7 and 100%"
        );
    }

    #[test]
    fn formats_times_like_messages() {
        assert_eq!(
            message_time("2016-07-08T18:16:31.1081039Z").unwrap(),
            "\u{200e}2016\u{200e}-\u{200e}07\u{200e}-\u{200e}08T18:16:31.108103900Z"
        );
    }
}
