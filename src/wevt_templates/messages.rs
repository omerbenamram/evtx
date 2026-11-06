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
use std::sync::Arc;

use bumpalo::Bump;
use thiserror::Error;

use super::WevtTemplateExtractError;
use super::extract::{RT_MESSAGETABLE, extract_resources, extract_wevt_template_resources};
use super::manifest::{CrimManifest, EventDefinition, WevtManifestError};
use super::normalize_guid;
use super::types::ResourceIdentifier;
use crate::binxml::compiled::MAX_RECORD_OUTPUT;
use crate::binxml::value_variant::BinXmlValue;
use crate::model::ir::{Element, ElementId, IrTree, Node, Text};
use crate::mta::RenderingInfo;
use crate::mta::{decode_utf16, narrow, number_value, value_number};
use crate::utils::bytes::{read_u16_le, read_u32_le};

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
        let u32_at = |at: usize| read_u32_le(data, at);
        let malformed = MessageCatalogError::MessageTable;
        let blocks = u32_at(0).ok_or(malformed("no block count"))? as usize;
        let mut messages = HashMap::new();
        // A real table stores each entry once, so its entries fit in it. A crafted one could
        // point every block at the same entries, to decode them once for each block: only as
        // many bytes of entries as the table has are read.
        let mut room = data.len();
        for block in 0..blocks.min(data.len() / 12) {
            let at = 4 + block * 12;
            let (Some(low), Some(high), Some(mut entry)) =
                (u32_at(at), u32_at(at + 4), u32_at(at + 8))
            else {
                return Err(malformed("block runs past the table"));
            };
            for id in low..=high {
                let entry_at = entry as usize;
                let (Some(length), Some(flags)) = (
                    read_u16_le(data, entry_at),
                    read_u16_le(data, entry_at.wrapping_add(2)),
                ) else {
                    return Err(malformed("entry runs past the table"));
                };
                let length = usize::from(length);
                if length < 4 {
                    return Err(malformed("entry shorter than its header"));
                }
                room = room
                    .checked_sub(length)
                    .ok_or(malformed("blocks share entries"))?;
                // `entry_at + 4` can't overflow: the header read above ends there.
                let text = entry_at
                    .checked_add(length)
                    .and_then(|end| data.get(entry_at + 4..end))
                    .ok_or(malformed("entry text runs past the table"))?;
                // `MESSAGE_RESOURCE_UNICODE`, then `MESSAGE_RESOURCE_UTF8`.
                let text = if flags & 1 != 0 {
                    String::from(decode_utf16(text))
                } else if flags & 2 != 0 {
                    String::from_utf8_lossy(text).into_owned()
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

    /// The number of messages in the table.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the table has no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// The input and output types of an event template's items, in order, shared by the events
/// that use the template.
type ItemTypes = Arc<[(u8, u8)]>;

/// The messages of one provider.
#[derive(Debug, Clone, Default)]
struct ProviderMessages {
    /// The tables to look message ids up in: the provider's `.mui` files, then its resource file.
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
    /// An empty catalog, to fill with [`MessageCatalog::add_path`] and the other `add_` methods.
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

    /// Adds the message tables of `files`, in order, and returns their indexes. Adds none
    /// when one of them fails to parse, so that its error doesn't leave the others unused.
    fn add_tables<'f>(
        &mut self,
        files: impl IntoIterator<Item = &'f [u8]>,
    ) -> Result<Vec<usize>, MessageCatalogError> {
        let tables = files
            .into_iter()
            .map(MessageTable::from_pe_bytes)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tables
            .into_iter()
            .filter_map(|table| self.add_table(table))
            .collect())
    }

    /// Adds the manifest providers of `resource_file` (a binary with a `WEVT_TEMPLATE`), with
    /// strings from its own message table and from `message_files` (its `.mui` files). Returns
    /// how many providers it added; a provider already in the catalog keeps its first strings,
    /// unless it has none. The `.mui` files' strings win over the resource file's, as they do
    /// for Windows' resource loader.
    pub fn add_provider_files(
        &mut self,
        resource_file: &[u8],
        message_files: &[&[u8]],
    ) -> Result<usize, MessageCatalogError> {
        self.add_providers(resource_file, || Ok(message_files.to_vec()))
    }

    /// [`MessageCatalog::add_provider_files`], reading the message files only when the resource
    /// file has providers that the catalog takes.
    fn add_providers<M: AsRef<[u8]>>(
        &mut self,
        resource_file: &[u8],
        message_files: impl FnOnce() -> Result<Vec<M>, MessageCatalogError>,
    ) -> Result<usize, MessageCatalogError> {
        let resources = extract_wevt_template_resources(resource_file)?;
        let manifests = resources
            .iter()
            .map(|manifest| CrimManifest::parse(&manifest.data))
            .collect::<Result<Vec<_>, _>>()?;
        // A provider already in the catalog with no strings takes them from a later copy.
        let is_new = |catalog: &Self, guid: &str| {
            catalog
                .providers
                .get(guid)
                .is_none_or(|provider| provider.tables.is_empty())
        };
        let any_new = manifests
            .iter()
            .flat_map(|m| &m.providers)
            .any(|provider| is_new(self, &normalize_guid(&provider.guid.to_string())));
        if !any_new {
            return Ok(0);
        }
        let message_files = message_files()?;
        let files = message_files
            .iter()
            .map(AsRef::as_ref)
            .chain(std::iter::once(resource_file));
        let tables = self.add_tables(files)?;
        let mut added = 0;
        for manifest in &manifests {
            for provider in &manifest.providers {
                let guid = normalize_guid(&provider.guid.to_string());
                if !is_new(self, &guid) {
                    continue;
                }
                let elements = &provider.wevt.elements;
                // Each template's item input and output types, by the template's offset.
                let input_types: HashMap<u32, ItemTypes> = elements
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
        let tables = self.add_tables(message_files.iter().copied())?;
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
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                // Only the folder given has to be readable; others are skipped like files.
                Err(e) if dir != path => {
                    log::debug!("skipping `{}`: {e}", dir.display());
                    continue;
                }
                Err(source) => return Err(MessageCatalogError::Io { path: dir, source }),
            };
            let mut paths: Vec<(PathBuf, bool)> = entries
                .filter_map(Result::ok)
                // Not following links to folders, which can loop (such as the `Application
                // Data` junction under `config\systemprofile`).
                .map(|e| (e.path(), e.file_type().is_ok_and(|t| t.is_dir())))
                .collect();
            paths.sort();
            for (path, is_dir) in paths {
                if is_dir {
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
        let read_mui = || {
            path.parent()
                .and_then(|parent| find_mui(parent, locale, name))
                .map(|mui| read(&mui))
                .transpose()
        };
        let result = read(path).and_then(|bytes| {
            if lower == "msobjs.dll" {
                let mui = read_mui()?;
                self.add_parameter_file(mui.as_deref().unwrap_or(&bytes))
            } else {
                // Only binaries with providers the catalog takes read their `.mui`.
                let message_files = || Ok(read_mui()?.into_iter().collect::<Vec<_>>());
                self.add_providers(&bytes, message_files).map(drop)
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
        tree: &IrTree<'_>,
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
                    &event.inserts(tree),
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
                .or_else(|| winmeta_task(event.task));
            // As `MtaFile::rendering_info`: no opcode for events from classic event sources.
            let opcode = if event.qualifiers == 0 {
                name(
                    provider
                        .opcodes
                        .get(&((u32::from(event.opcode) << 16) | u32::from(event.task)))
                        .or_else(|| provider.opcodes.get(&(u32::from(event.opcode) << 16))),
                )
                .or_else(|| winmeta_opcode(event.opcode))
            } else {
                None
            };
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
            .map(|template| self.format(template, &event.inserts(tree), &[], tables, bump));
        // The standard names, as for a manifest provider that names none of its own.
        Some(RenderingInfo {
            message,
            level: winmeta_level(event.level),
            task: winmeta_task(event.task),
            opcode: (event.qualifiers == 0)
                .then(|| winmeta_opcode(event.opcode))
                .flatten(),
            keywords: (0..64)
                .filter(|bit| event.keywords & (1 << bit) != 0)
                .filter_map(winmeta_keyword)
                .collect(),
        })
    }

    /// Fills the inserts of `template` with `inserts`, as `FormatMessage` does, and replaces the
    /// `%%<id>` parameters in them.
    ///
    /// Stops once the message is longer than a record may render ([`MAX_RECORD_OUTPUT`]),
    /// which then fails its record: a crafted record can fill an insert with parameters that
    /// each name a long message, and so make the message far larger than itself.
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
        while out.len() <= MAX_RECORD_OUTPUT
            && let Some(c) = chars.next()
        {
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
                        Some(insert) => {
                            let room = MAX_RECORD_OUTPUT.saturating_sub(out.len());
                            out.push_str(&self.parameters(&insert.text, tables, room));
                        }
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

    /// Replaces each `%%<id>` in an event data value with the parameter string `id`, until the
    /// value is longer than `limit` (the rest is then left as it is).
    fn parameters<'v>(&self, value: &'v str, tables: &[usize], limit: usize) -> Cow<'v, str> {
        if !value.contains("%%") {
            return Cow::Borrowed(value);
        }
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while out.len() <= limit
            && let Some(at) = rest.find("%%")
        {
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

/// The `.mui` of the binary `name` in the `locale` folder of `folder`. Windows names them in any
/// case, so on a case-sensitive filesystem (a copy of `System32` on Linux) the folder and the
/// file are matched ignoring ASCII case when the exact names aren't there.
fn find_mui(folder: &Path, locale: &str, name: &str) -> Option<PathBuf> {
    let file_name = format!("{name}.mui");
    let exact = folder.join(locale).join(&file_name);
    if exact.is_file() {
        return Some(exact);
    }
    let find = |folder: &Path, wanted: &str, want_dir: bool| {
        std::fs::read_dir(folder).ok()?.flatten().find_map(|entry| {
            let matches = entry.file_name().to_str()?.eq_ignore_ascii_case(wanted)
                && entry.file_type().ok()?.is_dir() == want_dir;
            matches.then(|| entry.path())
        })
    };
    let locale_folder = find(folder, locale, true)?;
    find(&locale_folder, &file_name, false)
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

fn winmeta_task(task: u16) -> Option<&'static str> {
    (task == 0).then_some("None")
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
    /// The value, when it is a non-negative integer.
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
    /// The record's `EventData` and `UserData` elements, whose values fill the message.
    data: Vec<ElementId>,
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
                "EventData" | "UserData" => fields.data.push(*id),
                "RenderingInfo" => has_rendering_info = true,
                _ => {}
            }
        }
        fields.provider_guid = guid_text;
        Some((fields, has_rendering_info))
    }

    /// The event's data values, in order: the `Data` elements of `EventData`, or the leaf
    /// elements under `UserData`, up to the [`MAX_INSERTS`] a message can use. Read only for a
    /// message the catalog has.
    fn inserts(&self, tree: &IrTree<'_>) -> Vec<Insert> {
        let mut inserts = Vec::new();
        for &id in &self.data {
            let element = tree.element(id);
            if element.name.as_str() == "EventData" {
                let room = MAX_INSERTS - inserts.len();
                let values = element_children(tree, element).take(room);
                inserts.extend(values.map(|child| insert(&child.children)));
            } else {
                collect_leaves(tree, element, &mut inserts);
            }
        }
        inserts
    }
}

/// The most inserts a message can use: `FormatMessage` numbers them `%1` to `%99`.
const MAX_INSERTS: usize = 99;

fn element_children<'e, 'a>(
    tree: &'e IrTree<'a>,
    element: &'e Element<'a>,
) -> impl Iterator<Item = &'e Element<'a>> {
    element.children.iter().filter_map(move |node| match node {
        Node::Element(id) => Some(tree.element(*id)),
        _ => None,
    })
}

/// Pushes the text of each leaf element under `element`, in document order, until `out` holds
/// [`MAX_INSERTS`]. A crafted record's tree can nest values tens of thousands of elements deep,
/// and place one value's elements in many places, so that it has far more paths to leaves than
/// nodes: this walks with a stack of its own, and stops at the inserts a message can use.
fn collect_leaves<'e, 'a>(tree: &'e IrTree<'a>, element: &'e Element<'a>, out: &mut Vec<Insert>) {
    let mut stack = vec![element.children.iter()];
    while out.len() < MAX_INSERTS
        && let Some(children) = stack.last_mut()
    {
        let Some(node) = children.next() else {
            stack.pop();
            continue;
        };
        let Node::Element(id) = node else { continue };
        let child = tree.element(*id);
        if child.has_element_child {
            stack.push(child.children.iter());
        } else {
            out.push(insert(&child.children));
        }
    }
}

fn insert(nodes: &[Node<'_>]) -> Insert {
    let number = match nodes {
        [Node::Value(value)] => value_number(value),
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
            Node::Text(Text::Utf16(s))
            | Node::CData(Text::Utf16(s))
            | Node::Value(BinXmlValue::StringType(s)) => {
                Cow::Owned(decode_utf16(s.as_bytes()).into())
            }
            Node::CData(Text::Utf8(s)) => Cow::Borrowed(*s),
            Node::Value(value) => Cow::Owned(value.to_string()),
            Node::CharRef(c) => Cow::Owned(char::from_u32(u32::from(*c)).unwrap_or('_').into()),
            Node::EntityRef(name) => Cow::Borrowed(match name.as_str() {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "apos" => "'",
                _ => "",
            }),
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
    let Some(info) = catalog.rendering_info(&fields, tree, bump) else {
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

    #[test]
    fn finds_a_mui_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("en-us")).unwrap();
        std::fs::write(dir.path().join("en-us").join("adtschema.dll.mui"), b"").unwrap();
        // On a case-insensitive filesystem (macOS, Windows) the exact path is found as it is.
        let found = find_mui(dir.path(), "en-US", "AdtSchema.dll").unwrap();
        assert!(found.is_file(), "{found:?}");
        assert_eq!(find_mui(dir.path(), "de-DE", "AdtSchema.dll"), None);
    }

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

    #[test]
    fn blocks_cannot_share_entries() {
        // 2,000 blocks of 6,000 ids each, all at the same 6,000 empty entries: 48 KB that made
        // 12 million messages.
        let (blocks, entries) = (2000u32, 6000u32);
        let mut data = blocks.to_le_bytes().to_vec();
        for block in 0..blocks {
            data.extend_from_slice(&(block * entries).to_le_bytes());
            data.extend_from_slice(&(block * entries + entries - 1).to_le_bytes());
            data.extend_from_slice(&(4 + 12 * blocks).to_le_bytes());
        }
        for _ in 0..entries {
            data.extend_from_slice(&[4, 0, 1, 0]);
        }
        assert!(MessageTable::parse(&data).is_err());
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
            catalog.parameters("%%100 then %%101, %%7 and 100%", &[], usize::MAX),
            "Yes then No, %%7 and 100%"
        );
    }

    #[test]
    fn messages_stop_past_the_record_cap() {
        // A 30,000-character parameter, named 1,000 times in each of two inserts.
        let long = "x".repeat(30_000);
        let mut catalog = catalog_with(&[&long]);
        catalog.parameter_tables.push(0);
        let bump = Bump::new();
        let inserts = [insert(&"%%100".repeat(1000)), insert(&"%%100".repeat(1000))];
        let message = catalog.format("%1%2", &inserts, &[], &[], &bump);
        assert!(message.len() > MAX_RECORD_OUTPUT);
        assert!(message.len() < MAX_RECORD_OUTPUT + 2 * 5000 + 30_000);
    }

    #[test]
    fn formats_times_like_messages() {
        assert_eq!(
            message_time("2016-07-08T18:16:31.1081039Z").unwrap(),
            "\u{200e}2016\u{200e}-\u{200e}07\u{200e}-\u{200e}08T18:16:31.108103900Z"
        );
    }
}
