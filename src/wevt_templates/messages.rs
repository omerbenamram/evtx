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
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bumpalo::Bump;
use thiserror::Error;

use super::WevtTemplateExtractError;
use super::extract::{RT_MESSAGETABLE, extract_resources, extract_wevt_template_resources};
use super::manifest::{CrimManifest, Provider, WevtManifestError};
use super::normalize_guid;
use super::types::{PeResource, ResourceIdentifier};
use crate::binxml::compiled::MAX_RECORD_OUTPUT;
use crate::binxml::value_variant::BinXmlValue;
use crate::model::ir::{Element, ElementId, IrTree, Node};
use crate::mta::RenderingInfo;
use crate::mta::{attribute, decode_utf16, narrow, number_value, text, value_number};
use crate::utils::bytes::{read_u16_le, read_u32_le};

/// The language id of `en-US`, preferred when a binary has message tables in several languages
/// and none in the one asked for.
const EN_US: u32 = 1033;

/// The language ids (`LANGID`) of common locale names.
const LOCALES: &[(&str, u32)] = &[
    ("ar-SA", 0x0401),
    ("bg-BG", 0x0402),
    ("zh-TW", 0x0404),
    ("cs-CZ", 0x0405),
    ("da-DK", 0x0406),
    ("de-DE", 0x0407),
    ("el-GR", 0x0408),
    ("en-US", 0x0409),
    ("fi-FI", 0x040B),
    ("fr-FR", 0x040C),
    ("he-IL", 0x040D),
    ("hu-HU", 0x040E),
    ("it-IT", 0x0410),
    ("ja-JP", 0x0411),
    ("ko-KR", 0x0412),
    ("nl-NL", 0x0413),
    ("nb-NO", 0x0414),
    ("pl-PL", 0x0415),
    ("pt-BR", 0x0416),
    ("ro-RO", 0x0418),
    ("ru-RU", 0x0419),
    ("hr-HR", 0x041A),
    ("sk-SK", 0x041B),
    ("sv-SE", 0x041D),
    ("th-TH", 0x041E),
    ("tr-TR", 0x041F),
    ("uk-UA", 0x0422),
    ("sl-SI", 0x0424),
    ("et-EE", 0x0425),
    ("lv-LV", 0x0426),
    ("lt-LT", 0x0427),
    ("zh-CN", 0x0804),
    ("en-GB", 0x0809),
    ("es-MX", 0x080A),
    ("pt-PT", 0x0816),
    ("sr-Latn-RS", 0x241A),
    ("zh-HK", 0x0C04),
    ("es-ES", 0x0C0A),
    ("fr-CA", 0x0C0C),
];

/// The language whose message tables a binary's are read in.
#[derive(Debug, Clone, Copy, Default)]
struct Language {
    /// The locale's `LANGID`, when [`LOCALES`] has it.
    lang_id: Option<u32>,
    /// Its primary language (the low 10 bits of a `LANGID`), also for a locale [`LOCALES`]
    /// only has another country of.
    primary: Option<u32>,
}

impl Language {
    fn of(locale: &str) -> Self {
        let lang_id = LOCALES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(locale))
            .map(|&(_, id)| id);
        let primary = lang_id
            .or_else(|| {
                LOCALES
                    .iter()
                    .find(|(name, _)| language_of(name).eq_ignore_ascii_case(language_of(locale)))
                    .map(|&(_, id)| id)
            })
            .map(primary_language);
        Language { lang_id, primary }
    }
}

/// The language part of a locale name, such as `de` of `de-DE`.
fn language_of(locale: &str) -> &str {
    locale.split(['-', '_']).next().unwrap_or_default()
}

fn primary_language(lang_id: u32) -> u32 {
    lang_id & 0x3FF
}

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
                    decode_utf16(text)
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

    /// Reads the message tables of a PE binary (`.dll`, `.exe` or `.mui`), preferring `en-US`
    /// when it has several languages. Empty when the binary has no message table.
    pub fn from_pe_bytes(pe_bytes: &[u8]) -> Result<Self, MessageCatalogError> {
        Self::read(pe_bytes, Language::default())
    }

    /// [`MessageTable::from_pe_bytes`], preferring the language of `locale` (such as `de-DE`),
    /// then `en-US`.
    pub fn from_pe_bytes_in(pe_bytes: &[u8], locale: &str) -> Result<Self, MessageCatalogError> {
        Self::read(pe_bytes, Language::of(locale))
    }

    fn read(pe_bytes: &[u8], language: Language) -> Result<Self, MessageCatalogError> {
        let resources = extract_resources(pe_bytes, &ResourceIdentifier::Id(RT_MESSAGETABLE))?;
        Self::from_resources(&resources, language)
    }

    /// Joins the `RT_MESSAGETABLE` resources in one language: `language`'s, or one of the same
    /// primary language, then `en-US`, then the first one's. A binary can have several.
    fn from_resources(
        resources: &[PeResource],
        language: Language,
    ) -> Result<Self, MessageCatalogError> {
        let find = |matches: &dyn Fn(u32) -> bool| resources.iter().find(|r| matches(r.lang_id));
        let chosen = language
            .lang_id
            .and_then(|id| find(&|lang_id| lang_id == id))
            .or_else(|| {
                let primary = language.primary?;
                find(&|lang_id| primary_language(lang_id) == primary)
            })
            .or_else(|| find(&|lang_id| lang_id == EN_US))
            .or_else(|| resources.first());
        let Some(chosen) = chosen else {
            return Ok(MessageTable::default());
        };
        let mut table = MessageTable::default();
        for resource in resources.iter().filter(|r| r.lang_id == chosen.lang_id) {
            for (id, message) in Self::parse(&resource.data)?.messages {
                table.messages.entry(id).or_insert(message);
            }
        }
        Ok(table)
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

/// What a manifest says about one event's message.
#[derive(Debug, Clone)]
struct EventMessage {
    level: u8,
    opcode: u8,
    task: u16,
    keywords: u64,
    message_identifier: u32,
    item_types: ItemTypes,
}

/// The messages of one provider.
#[derive(Debug, Clone, Default)]
struct ProviderMessages {
    /// The tables to look message ids up in: the provider's `.mui` files, then its resource file.
    tables: Vec<usize>,
    /// Events by `(event id, version)`, with the rest of each event's descriptor to pick
    /// between events that share both.
    events: HashMap<(u16, u8), Vec<EventMessage>>,
    levels: HashMap<u32, u32>,
    tasks: HashMap<u32, u32>,
    /// By `(opcode << 16) | task`, with task 0 for opcodes not tied to a task.
    opcodes: HashMap<u32, u32>,
    /// By keyword mask.
    keywords: HashMap<u64, u32>,
}

impl ProviderMessages {
    /// A manifest provider's message ids, without tables yet.
    fn of(provider: &Provider<'_>) -> Self {
        let elements = &provider.wevt.elements;
        // Each template's item input and output types, by the template's offset.
        let item_types: HashMap<u32, ItemTypes> = elements
            .templates
            .iter()
            .flat_map(|t| &t.templates)
            .map(|t| {
                let types = t.items.iter().map(|i| (i.input_type, i.output_type));
                (t.offset, types.collect())
            })
            .collect();
        let mut messages = ProviderMessages::default();
        for event in elements.events.iter().flat_map(|e| &e.events) {
            let item_types = event
                .template_offset
                .and_then(|offset| item_types.get(&offset).cloned())
                .unwrap_or_default();
            messages
                .events
                .entry((event.identifier, event.version))
                .or_default()
                .push(EventMessage {
                    level: event.level,
                    opcode: event.opcode,
                    task: event.task,
                    keywords: event.keywords,
                    message_identifier: event.message_identifier,
                    item_types,
                });
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
        messages
    }

    /// The event a record is, by its whole descriptor, or the first with its id and version.
    fn event(&self, event: &EventFields<'_>) -> Option<&EventMessage> {
        let candidates = self.events.get(&(event.event_id, event.version))?;
        candidates
            .iter()
            .find(|m| {
                m.level == event.level
                    && m.opcode == event.opcode
                    && m.task == event.task
                    && m.keywords == event.keywords
            })
            .or_else(|| candidates.first())
    }
}

/// Event messages from provider binaries. See [the module docs](self).
#[derive(Debug, Clone, Default)]
pub struct MessageCatalog {
    tables: Vec<MessageTable>,
    /// Manifest providers by normalized GUID.
    providers: HashMap<String, ProviderMessages>,
    /// Classic event sources, by ASCII-lowercased name.
    sources: HashMap<String, Vec<usize>>,
    /// The tables that `%%<id>` parameters are looked up in, such as `msobjs.dll`'s for the
    /// Security log (Windows' `ParameterMessageFile`).
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

    /// Adds the message tables of `files`, in order, and returns their indexes. A file whose
    /// tables don't parse is skipped, with a warning.
    fn add_tables<'f>(
        &mut self,
        files: impl IntoIterator<Item = &'f [u8]>,
        language: Language,
    ) -> Vec<usize> {
        files
            .into_iter()
            .filter_map(|file| match MessageTable::read(file, language) {
                Ok(table) => self.add_table(table),
                Err(e) => {
                    log::warn!("skipping a message table: {e}");
                    None
                }
            })
            .collect()
    }

    /// Whether the catalog takes the provider `guid` (normalized): it doesn't have it, or has
    /// it with no strings, which a later copy then gives.
    fn takes(&self, guid: &str) -> bool {
        self.providers
            .get(guid)
            .is_none_or(|provider| provider.tables.is_empty())
    }

    /// The providers of `resource_file`'s manifests that the catalog takes, by normalized GUID.
    fn new_providers(
        &self,
        resource_file: &[u8],
    ) -> Result<Vec<(String, ProviderMessages)>, MessageCatalogError> {
        let mut providers = Vec::new();
        for resource in extract_wevt_template_resources(resource_file)? {
            for provider in &CrimManifest::parse(&resource.data)?.providers {
                let guid = normalize_guid(&provider.guid.to_string());
                if self.takes(&guid) {
                    providers.push((guid, ProviderMessages::of(provider)));
                }
            }
        }
        Ok(providers)
    }

    /// Adds `providers` (from [`MessageCatalog::new_providers`]) with strings from `tables`, and
    /// returns how many it added.
    fn insert_providers(
        &mut self,
        providers: Vec<(String, ProviderMessages)>,
        tables: Vec<usize>,
    ) -> usize {
        let mut added = 0;
        for (guid, mut messages) in providers {
            // A manifest can repeat a provider.
            if self.takes(&guid) {
                messages.tables = tables.clone();
                self.providers.insert(guid, messages);
                added += 1;
            }
        }
        added
    }

    /// Adds the manifest providers of `resource_file` (a binary with a `WEVT_TEMPLATE`), with
    /// strings from its own message table and from `message_files` (its `.mui` files). Returns
    /// how many providers it added; a provider already in the catalog keeps its first strings,
    /// unless it has none. The `.mui` files' strings win over the resource file's, as they do
    /// for Windows' resource loader. A message file whose table doesn't parse is skipped, with a
    /// warning.
    pub fn add_provider_files(
        &mut self,
        resource_file: &[u8],
        message_files: &[&[u8]],
    ) -> Result<usize, MessageCatalogError> {
        let providers = self.new_providers(resource_file)?;
        if providers.is_empty() {
            return Ok(0);
        }
        let files = message_files.iter().copied();
        let tables = self.add_tables(
            files.chain(std::iter::once(resource_file)),
            Language::default(),
        );
        Ok(self.insert_providers(providers, tables))
    }

    /// Adds a classic event source (an event log provider without a manifest, matched by its
    /// `Provider/@Name`), with strings from its message files. Its events' message ids are
    /// `(Qualifiers << 16) | EventID`. A message file whose table doesn't parse is skipped, with
    /// a warning.
    pub fn add_classic_source(&mut self, name: &str, message_files: &[&[u8]]) {
        let tables = self.add_tables(message_files.iter().copied(), Language::default());
        self.sources
            .entry(name.to_ascii_lowercase())
            .or_default()
            .extend(tables);
    }

    /// Adds a parameter message file, whose strings replace `%%<id>` in messages and event data,
    /// as `msobjs.dll` does for the Security log.
    pub fn add_parameter_file(&mut self, pe_bytes: &[u8]) -> Result<(), MessageCatalogError> {
        self.add_parameter_table(pe_bytes, Language::default())
    }

    fn add_parameter_table(
        &mut self,
        pe_bytes: &[u8],
        language: Language,
    ) -> Result<(), MessageCatalogError> {
        if let Some(index) = self.add_table(MessageTable::read(pe_bytes, language)?) {
            self.parameter_tables.push(index);
        }
        Ok(())
    }

    /// Builds a catalog from a copy of a Windows binaries folder, such as `C:\Windows\System32`.
    /// Every `.dll`, `.exe` and `.sys` under `dir` with a manifest adds its providers, with
    /// strings from the `.mui` file of the same name in the `locale` subfolder of its folder
    /// (`en-US` when `None`), and from its own message tables in that locale's language when it
    /// has them. `msobjs.dll` (or its `.mui`) is added as a parameter file.
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
        let language = Language::of(locale);
        let path = path.as_ref();
        if path.is_file() {
            let locale_folder = path
                .parent()
                .and_then(|parent| locale_folder(parent, locale));
            return self.add_binary(path, locale_folder.as_deref(), language, true);
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
            let locale_folder = locale_folder(&dir, locale);
            for (path, is_dir) in paths {
                if is_dir {
                    pending.push(path);
                    continue;
                }
                self.add_binary(&path, locale_folder.as_deref(), language, false)?;
            }
        }
        Ok(())
    }

    /// Adds one binary found in a folder, or given on its own (`strict`), whose errors are then
    /// returned instead of skipped. Its `.mui` is looked for in `locale_folder`.
    ///
    /// Each binary is read whole, though only its resources are used.
    fn add_binary(
        &mut self,
        path: &Path,
        locale_folder: Option<&Path>,
        language: Language,
        strict: bool,
    ) -> Result<(), MessageCatalogError> {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return Ok(());
        };
        let lower = name.to_ascii_lowercase();
        if !strict
            && !(lower.ends_with(".dll") || lower.ends_with(".exe") || lower.ends_with(".sys"))
        {
            return Ok(());
        }
        // A `.mui` that can't be read is skipped, as one that doesn't parse is.
        let read_mui = || {
            let mui = find_mui(locale_folder?, name)?;
            std::fs::read(&mui)
                .inspect_err(|e| log::warn!("skipping `{}`: {e}", mui.display()))
                .ok()
        };
        let result = std::fs::read(path)
            .map_err(|source| MessageCatalogError::Io {
                path: path.to_path_buf(),
                source,
            })
            .and_then(|bytes| {
                if lower == "msobjs.dll" {
                    let mui = read_mui();
                    return self.add_parameter_table(mui.as_deref().unwrap_or(&bytes), language);
                }
                let providers = self.new_providers(&bytes)?;
                // Only binaries with providers the catalog takes read their `.mui`.
                if !providers.is_empty() {
                    let mui = read_mui();
                    let files = mui.as_deref().into_iter().chain([&bytes[..]]);
                    let tables = self.add_tables(files, language);
                    self.insert_providers(providers, tables);
                }
                Ok(())
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

    /// The parameter string of a `%%<id>` (given its digits), from the parameter files.
    fn parameter(&self, digits: &str) -> Option<&str> {
        let id = digits.parse().ok()?;
        self.lookup(&self.parameter_tables, id).map(trim_message)
    }

    /// The manifest provider with GUID `guid` (in any case, with or without braces), without
    /// allocating for a GUID in the usual form.
    fn provider(&self, guid: &str) -> Option<&ProviderMessages> {
        if self.providers.is_empty() {
            return None;
        }
        let guid = guid.trim().trim_start_matches('{').trim_end_matches('}');
        let mut key = [0u8; 36];
        if guid.len() != key.len() {
            return self.providers.get(&normalize_guid(guid));
        }
        key.copy_from_slice(guid.as_bytes());
        key.make_ascii_lowercase();
        // Lowercasing ASCII bytes keeps UTF-8 valid.
        self.providers.get(std::str::from_utf8(&key).ok()?)
    }

    /// The strings of an event, or `None` when its provider isn't in the catalog. Allocates the
    /// formatted message in `bump`.
    fn rendering_info<'b>(
        &'b self,
        event: &EventFields<'_>,
        tree: &IrTree<'_>,
        bump: &'b Bump,
    ) -> Option<RenderingInfo<'b>> {
        let provider = self.provider(&event.provider_guid);
        // The tables, the event's message id, and its template's item types.
        let (tables, message_id, item_types) = match provider {
            Some(provider) => {
                let message = provider.event(event);
                (
                    &provider.tables[..],
                    message.map(|m| m.message_identifier),
                    message.map_or(&[][..], |m| &m.item_types[..]),
                )
            }
            None => {
                if self.sources.is_empty() {
                    return None;
                }
                let tables = self
                    .sources
                    .get(&event.provider_name.to_ascii_lowercase())?;
                let id = (u32::from(event.qualifiers) << 16) | u32::from(event.event_id);
                (&tables[..], Some(id), &[][..])
            }
        };
        let message = message_id
            .and_then(|id| self.lookup(tables, id))
            .map(|template| {
                let (inserts, flat) = event.inserts(tree);
                // Item types are by the template's item, which the inserts are only when they
                // are the `Data` elements of one `EventData`, one for each item.
                let item_types = if flat && inserts.len() == item_types.len() {
                    item_types
                } else {
                    &[]
                };
                self.format(template, &inserts, item_types, bump)
            });
        // A name the provider gives, or else the standard one; a classic source gives none.
        let name = |id: Option<&u32>| id.and_then(|&id| self.lookup(tables, id)).map(trim_message);
        let level = name(provider.and_then(|p| p.levels.get(&u32::from(event.level))))
            .or_else(|| winmeta_level(event.level));
        let task = name(provider.and_then(|p| p.tasks.get(&u32::from(event.task))))
            .or_else(|| winmeta_task(event.task));
        // As `MtaFile::rendering_info`: no opcode for events from classic event sources.
        let opcode = (event.qualifiers == 0)
            .then(|| {
                let opcode = u32::from(event.opcode) << 16;
                let id = provider.and_then(|p| {
                    p.opcodes
                        .get(&(opcode | u32::from(event.task)))
                        .or_else(|| p.opcodes.get(&opcode))
                });
                name(id).or_else(|| winmeta_opcode(event.opcode))
            })
            .flatten();
        let keywords = (0..64)
            .filter(|bit| event.keywords & (1 << bit) != 0)
            .filter_map(|bit| {
                name(provider.and_then(|p| p.keywords.get(&(1u64 << bit))))
                    .or_else(|| winmeta_keyword(bit))
            })
            .collect();
        Some(RenderingInfo {
            message,
            level,
            task,
            opcode,
            keywords,
        })
    }

    /// Fills the inserts of `template` with `inserts`, as `FormatMessage` does, and replaces the
    /// `%%<id>` parameters in the template and in the inserts.
    ///
    /// Stops once the message is longer than a record may render ([`MAX_RECORD_OUTPUT`]),
    /// which then fails its record: a crafted record can fill an insert with parameters that
    /// each name a long message, and so make the message far larger than itself.
    fn format<'b>(
        &self,
        template: &str,
        inserts: &[Insert<'_>],
        item_types: &[(u8, u8)],
        bump: &'b Bump,
    ) -> &'b str {
        let mut out = bumpalo::collections::String::with_capacity_in(template.len(), bump);
        let mut rest = template;
        while out.len() <= MAX_RECORD_OUTPUT {
            let Some(at) = rest.find('%') else {
                out.push_str(rest);
                break;
            };
            out.push_str(&rest[..at]);
            let escape = &rest[at + 1..];
            let Some(c) = escape.chars().next() else {
                out.push('%');
                break;
            };
            // The bytes of `escape` that this escape uses.
            let mut used = c.len_utf8();
            match c {
                '1'..='9' => {
                    // `%1` to `%99`.
                    used = escape
                        .bytes()
                        .take(2)
                        .take_while(u8::is_ascii_digit)
                        .count();
                    let index = escape[..used].parse::<usize>().expect("digits") - 1;
                    // A printf-style format, such as `!x!`.
                    if escape[used..].starts_with('!') {
                        used = escape[used + 1..]
                            .find('!')
                            .map_or(escape.len(), |end| used + end + 2);
                    }
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
                            let _ = write!(out, "0x{number:X}");
                        }
                        Some(insert) => {
                            let room = MAX_RECORD_OUTPUT.saturating_sub(out.len());
                            out.push_str(&self.parameters(&insert.text, room));
                        }
                        None => out.push_str(&rest[at..at + 1 + used]),
                    }
                }
                // `%%`, or a parameter `%%<id>`, which stays as written when it isn't found.
                '%' => {
                    let digits = escape[1..].bytes().take_while(u8::is_ascii_digit).count();
                    if digits == 0 {
                        out.push('%');
                    } else {
                        used += digits;
                        match self.parameter(&escape[1..used]) {
                            Some(parameter) => out.push_str(parameter),
                            None => out.push_str(&rest[at..at + 1 + used]),
                        }
                    }
                }
                '0' => break,
                'n' => out.push_str("\r\n"),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push(' '),
                other => out.push(other),
            }
            rest = &escape[used..];
        }
        let length = trim_message(&out).len();
        out.truncate(length);
        out.into_bump_str()
    }

    /// Replaces each `%%<id>` in an event data value with the parameter string `id`, until the
    /// value is longer than `limit` (the rest is then left as it is).
    fn parameters<'v>(&self, value: &'v str, limit: usize) -> Cow<'v, str> {
        if self.parameter_tables.is_empty() || !value.contains("%%") {
            return Cow::Borrowed(value);
        }
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while out.len() <= limit
            && let Some(at) = rest.find("%%")
        {
            out.push_str(&rest[..at]);
            let digits = rest[at + 2..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            let written = &rest[at..at + 2 + digits];
            out.push_str(self.parameter(&written[2..]).unwrap_or(written));
            rest = &rest[at + 2 + digits..];
        }
        out.push_str(rest);
        Cow::Owned(out)
    }
}

/// The `locale` folder in `folder`. Windows names them in any case, so on a case-sensitive
/// filesystem (a copy of `System32` on Linux) it is matched ignoring ASCII case when the exact
/// name isn't there.
fn locale_folder(folder: &Path, locale: &str) -> Option<PathBuf> {
    // A bare file name's folder is the current one.
    let folder = if folder.as_os_str().is_empty() {
        Path::new(".")
    } else {
        folder
    };
    let exact = folder.join(locale);
    if exact.is_dir() {
        return Some(exact);
    }
    find_ignoring_case(folder, locale, true)
}

/// The `.mui` of the binary `name` in `locale_folder`, matched as [`locale_folder`] is.
fn find_mui(locale_folder: &Path, name: &str) -> Option<PathBuf> {
    let file_name = format!("{name}.mui");
    let exact = locale_folder.join(&file_name);
    if exact.is_file() {
        return Some(exact);
    }
    find_ignoring_case(locale_folder, &file_name, false)
}

fn find_ignoring_case(folder: &Path, wanted: &str, want_dir: bool) -> Option<PathBuf> {
    std::fs::read_dir(folder).ok()?.flatten().find_map(|entry| {
        let matches = entry.file_name().to_str()?.eq_ignore_ascii_case(wanted)
            && entry.file_type().ok()?.is_dir() == want_dir;
        matches.then(|| entry.path())
    })
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
        49 => "WDI Context",
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
struct Insert<'t> {
    text: Cow<'t, str>,
    /// The value, when it is a non-negative integer.
    number: Option<u64>,
}

/// The fields of a record's tree that select and fill its message.
#[derive(Debug, Default)]
struct EventFields<'t> {
    provider_name: Cow<'t, str>,
    /// Empty when the provider has none.
    provider_guid: Cow<'t, str>,
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
    /// `None` when the record gets no `RenderingInfo`: its root isn't `Event`, or it has a
    /// `RenderingInfo` already. As for an `.MTA` file, the first `System` holds the fields.
    fn of(tree: &'t IrTree<'_>) -> Option<Self> {
        let root = tree.root_element();
        if root.name.as_str() != "Event" {
            return None;
        }
        let mut fields = EventFields::default();
        let mut system = None;
        for node in &root.children {
            let Node::Element(id) = node else { continue };
            match tree.element(*id).name.as_str() {
                "System" => system = system.or(Some(*id)),
                "EventData" | "UserData" => fields.data.push(*id),
                "RenderingInfo" => return None,
                _ => {}
            }
        }
        let Some(system) = system else {
            return Some(fields);
        };
        // A record's tree has no substitutions, so `attribute` gives the first one's value.
        let attribute = |element, name| attribute(element, name).unwrap_or_default();
        for child in element_children(tree, tree.element(system)) {
            let number = || number_value(&child.children);
            match child.name.as_str() {
                "Provider" => {
                    fields.provider_name = text(attribute(child, "Name"));
                    fields.provider_guid = text(attribute(child, "Guid"));
                }
                "EventID" => {
                    fields.event_id = narrow(number());
                    fields.qualifiers = narrow(number_value(attribute(child, "Qualifiers")));
                }
                "Version" => fields.version = narrow(number()),
                "Level" => fields.level = narrow(number()),
                "Task" => fields.task = narrow(number()),
                "Opcode" => fields.opcode = narrow(number()),
                "Keywords" => fields.keywords = number().unwrap_or_default(),
                _ => {}
            }
        }
        Some(fields)
    }

    /// The event's data values, in order: the `Data` elements of `EventData`, or the leaf
    /// elements under `UserData`, up to the [`MAX_INSERTS`] a message can use. Read only for a
    /// message the catalog has. With whether they are the children of one `EventData`, each a
    /// value of its own.
    fn inserts<'a>(&self, tree: &'t IrTree<'a>) -> (Vec<Insert<'a>>, bool) {
        let mut inserts = Vec::new();
        let mut flat = self.data.len() == 1;
        for &id in &self.data {
            let element = tree.element(id);
            if element.name.as_str() == "EventData" {
                let room = MAX_INSERTS - inserts.len();
                for child in element_children(tree, element).take(room) {
                    flat &= !child.has_element_child;
                    inserts.push(insert(&child.children));
                }
            } else {
                flat = false;
                collect_leaves(tree, element, &mut inserts);
            }
        }
        (inserts, flat)
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
fn collect_leaves<'e, 'a>(
    tree: &'e IrTree<'a>,
    element: &'e Element<'a>,
    out: &mut Vec<Insert<'a>>,
) {
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

fn insert<'a>(nodes: &[Node<'a>]) -> Insert<'a> {
    let number = match nodes {
        [Node::Value(value)] => value_number(value),
        _ => None,
    };
    let text = text(nodes);
    let text = match nodes {
        [Node::Value(BinXmlValue::FileTimeType(_) | BinXmlValue::SysTimeType(_))] => {
            match message_time(&text) {
                Some(time) => Cow::Owned(time),
                None => text,
            }
        }
        _ => text,
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

/// Adds a `RenderingInfo` element to a record's tree from `catalog`, unless its provider isn't
/// in the catalog or it has one already.
pub(crate) fn add_rendering_info<'a>(
    tree: &mut IrTree<'a>,
    catalog: &'a MessageCatalog,
    bump: &'a Bump,
) {
    if catalog.providers.is_empty() && catalog.sources.is_empty() {
        return;
    }
    let Some(fields) = EventFields::of(tree) else {
        return;
    };
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
        let folder = locale_folder(dir.path(), "en-US").unwrap();
        let found = find_mui(&folder, "AdtSchema.dll").unwrap();
        assert!(found.is_file(), "{found:?}");
        assert_eq!(locale_folder(dir.path(), "de-DE"), None);
    }

    #[test]
    fn a_bare_file_name_looks_in_the_current_folder() {
        // `add_path("adtschema.dll")` gives the folder `""`, which `read_dir` refuses. Tests run
        // in the crate's folder, which has `src`.
        let found = locale_folder(Path::new(""), "SRC").unwrap();
        assert!(found.is_dir(), "{found:?}");
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

    fn resource(lang_id: u32, id: u32, messages: &[&str]) -> PeResource {
        PeResource {
            resource: ResourceIdentifier::Id(id),
            lang_id,
            data: table(100, messages),
        }
    }

    #[test]
    fn reads_the_tables_of_the_locale() {
        let resources = [
            resource(EN_US, 1, &["English"]),
            resource(0x0407, 1, &["Deutsch"]),
            resource(0x0407, 2, &["Deutsch", "Zwei"]),
        ];
        let read = |locale| MessageTable::from_resources(&resources, Language::of(locale)).unwrap();
        // Both of the German tables, the first one's strings first.
        let german = read("de-DE");
        assert_eq!(
            (german.get(100), german.get(101)),
            (Some("Deutsch"), Some("Zwei"))
        );
        // Another country of a language takes its tables, then `en-US`, then the first.
        assert_eq!(read("de-AT").get(100), Some("Deutsch"));
        assert_eq!(read("fr-FR").get(100), Some("English"));
        let resources = [resource(0x0407, 1, &["Deutsch"])];
        let table = MessageTable::from_resources(&resources, Language::of("xx-YY")).unwrap();
        assert_eq!(table.get(100), Some("Deutsch"));
    }

    #[test]
    fn a_bad_message_file_leaves_the_provider_its_own_strings() {
        let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/dlls/adtschema.dll");
        let dll = std::fs::read(dll).unwrap();
        let mut catalog = MessageCatalog::new();
        let added = catalog
            .add_provider_files(&dll, &[b"not a PE file"])
            .unwrap();
        assert_eq!(added, 1);
        let provider = catalog.providers.values().next().unwrap();
        assert!(!provider.tables.is_empty());
        assert!(catalog.lookup(&provider.tables, 4624).is_some());
    }

    fn catalog_with(messages: &[&str]) -> MessageCatalog {
        let mut catalog = MessageCatalog::new();
        let table = MessageTable::parse(&table(100, messages)).unwrap();
        catalog.add_table(table);
        catalog
    }

    fn insert(text: &str) -> Insert<'_> {
        Insert {
            text: Cow::Borrowed(text),
            number: None,
        }
    }

    #[test]
    fn formats_like_format_message() {
        let catalog = MessageCatalog::new();
        let bump = Bump::new();
        let inserts = [insert("alice"), insert("C:\\x"), insert("3")];
        let format = |template| catalog.format(template, &inserts, &[], &bump);
        assert_eq!(
            format("User:%t%1%nPath: %2!s!.%n%n"),
            "User:\talice\r\nPath: C:\\x."
        );
        assert_eq!(format("100%% sure%b%3%0 cut"), "100% sure 3");
        // An insert without a value stays as written, so does an unknown escape.
        assert_eq!(format("%1 and %12!d! %q"), "alice and %12!d! q");
        assert_eq!(format("%4!x"), "%4!x");
        assert_eq!(format("50%"), "50%");
    }

    #[test]
    fn formats_hex_numbers_by_output_type() {
        let catalog = MessageCatalog::new();
        let bump = Bump::new();
        let inserts = [
            Insert {
                text: Cow::Borrowed("999"),
                number: Some(999),
            },
            Insert {
                text: Cow::Borrowed("0x1c8"),
                number: Some(0x1c8),
            },
        ];
        // `win:UInt64` shown as `win:HexInt64`, and a `win:Pointer` shown as `win:HexInt64`.
        let types = [(10, 19), (INTYPE_POINTER, 19)];
        assert_eq!(
            catalog.format("%1 %2", &inserts, &types, &bump),
            "0x3E7 0x1c8"
        );
    }

    #[test]
    fn replaces_parameters() {
        let mut catalog = catalog_with(&["Yes\r\n", "No\r\n"]);
        catalog.parameter_tables.push(0);
        assert_eq!(
            catalog.parameters("%%100 then %%101, %%7 and 100%", usize::MAX),
            "Yes then No, %%7 and 100%"
        );
    }

    #[test]
    fn parameters_come_from_parameter_files_only() {
        // Table 0 is a provider's, table 1 a parameter file's.
        let mut catalog = catalog_with(&["Provider"]);
        catalog.add_table(MessageTable::parse(&table(100, &["Parameter"])).unwrap());
        let bump = Bump::new();
        assert_eq!(catalog.parameters("%%100", usize::MAX), "%%100");
        assert_eq!(catalog.format("%%100", &[], &[], &bump), "%%100");
        catalog.parameter_tables.push(1);
        assert_eq!(catalog.parameters("%%100", usize::MAX), "Parameter");
        // In the message itself too, where `%%` not before digits is `%`.
        assert_eq!(
            catalog.format("%%100 at 5%%.", &[], &[], &bump),
            "Parameter at 5%."
        );
    }

    #[test]
    fn messages_stop_past_the_record_cap() {
        // A 30,000-character parameter, named 1,000 times in each of two inserts.
        let long = "x".repeat(30_000);
        let mut catalog = catalog_with(&[&long]);
        catalog.parameter_tables.push(0);
        let bump = Bump::new();
        let (first, second) = ("%%100".repeat(1000), "%%100".repeat(1000));
        let inserts = [insert(&first), insert(&second)];
        let message = catalog.format("%1%2", &inserts, &[], &bump);
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

    #[test]
    fn names_the_standard_keywords() {
        let names: Vec<_> = (48..=55).filter_map(winmeta_keyword).collect();
        assert_eq!(
            names,
            [
                "Response Time",
                "WDI Context",
                "WDI Diag",
                "SQM",
                "Audit Failure",
                "Audit Success",
                "Correlation Hint",
                "Classic"
            ]
        );
    }

    #[test]
    fn finds_providers_by_any_spelling_of_their_guid() {
        let mut catalog = MessageCatalog::new();
        let guid = "54849625-5478-4994-a5ba-3e3b0328c30d";
        catalog
            .providers
            .insert(guid.to_string(), ProviderMessages::default());
        for spelling in [
            guid,
            "{54849625-5478-4994-A5BA-3E3B0328C30D}",
            " {54849625-5478-4994-a5ba-3e3b0328c30d} ",
        ] {
            assert!(catalog.provider(spelling).is_some(), "{spelling}");
        }
        assert!(catalog.provider("").is_none());
    }
}
