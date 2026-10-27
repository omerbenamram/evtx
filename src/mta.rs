//! Localized event strings from `.MTA` files (`LocaleMetaData`).
//!
//! An `.evtx` file holds event data but none of the strings needed to display it. When Windows
//! saves a log with display information (Event Viewer's *Save All Events As*, or `wevtutil epl`
//! followed by `wevtutil al`), it also writes those strings, for one locale, to
//! `LocaleMetaData\<log name>_<LCID>.MTA` next to the `.evtx`: the message of every event, and
//! the level, task, opcode and keyword names of every publisher in the log. MS-EVEN6 calls the
//! pair a localized log (section 3.1.1.7).
//!
//! [`MtaFile`] parses such a file. With [`ParserSettings::mta`](crate::ParserSettings::mta) set,
//! every record whose publisher is in the file gets a `RenderingInfo` element after its event
//! data, shaped like the one Windows adds to forwarded events. Strings the file does not have
//! are left out:
//!
//! ```xml
//! <RenderingInfo>
//!   <Message>An account was successfully logged on. ...</Message>
//!   <Level>Log Always</Level>
//!   <Task>Logon</Task>
//!   <Opcode>Info</Opcode>
//!   <Keywords>
//!     <Keyword>Audit Success</Keyword>
//!   </Keywords>
//! </RenderingInfo>
//! ```
//!
//! ```rust
//! use std::sync::Arc;
//! use evtx::{EvtxParser, ParserSettings, mta::MtaFile};
//!
//! # let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/mta");
//! let mta = MtaFile::from_path(samples.join("LocaleMetaData/setup-export_1033.MTA"))?;
//! let settings = ParserSettings::new().mta(Some(Arc::new(mta)));
//! let mut parser = EvtxParser::from_path(samples.join("setup-export.evtx"))?
//!     .with_configuration(settings);
//! let record = parser.records_json_value().next().unwrap()?;
//! assert_eq!(
//!     record.data["Event"]["RenderingInfo"]["Message"],
//!     "Initiating changes for package KB5020683. Current state is Absent. \
//!      Target state is Installed. Client id: WindowsUpdateAgent."
//! );
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! `docs/mta.md` describes the file format and how Windows matches its strings to events.

use ahash::AHashMap as HashMap;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use bumpalo::Bump;
use thiserror::Error;

use crate::binxml::value_variant::BinXmlValue;
use crate::model::ir::{Element, ElementId, IrArena, IrTree, Name, Node, Text};
use crate::utils::bytes::{read_array, read_u32_le, read_u64_le};

const MAGIC: &[u8; 8] = b"MTAFile\0";
const HEADER_SIZE: usize = 24;
/// Section descriptor: offset and size (u64 each), name length in bytes (u32), then the name.
const DESCRIPTOR_SIZE: usize = 20;
/// Page header: 64 bytes of index data, the page number (u64), the offsets of the next page and
/// of this one (u64 each), then the offsets of up to 100 records (u64, 0 for an unused slot).
const PAGE_HEADER_SIZE: usize = 888;
const PAGE_NEXT: usize = 72;
const PAGE_SELF: usize = 80;
const PAGE_RECORDS: usize = 88;
/// Record header: entry index and payload size (u32 each).
const RECORD_HEADER_SIZE: usize = 8;

/// Name of the publisher entry holding the standard (winmeta) level, task, opcode and keyword
/// names, used when an event's own publisher does not name a value.
pub const DEFAULT_PUBLISHER: &str = "DEFAULT_PUBLISHER";

/// An error while reading an `.MTA` file.
#[derive(Debug, Error)]
pub enum MtaError {
    #[error("failed to read `{}`", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("not an MTA file (magic {magic:02x?})")]
    InvalidMagic { magic: [u8; 8] },

    #[error("unsupported MTA version {major}.{minor}")]
    UnsupportedVersion { major: u32, minor: u32 },

    #[error("MTA file has no {name} section")]
    MissingSection { name: &'static str },

    #[error("MTA {what} at offset {offset:#x} is truncated or out of bounds")]
    Corrupt { what: &'static str, offset: u64 },

    #[error("MTA {section} section has a page chain that does not end")]
    PageLoop { section: &'static str },
}

/// The localized strings of one exported log, for one locale.
///
/// Parse one with [`MtaFile::from_path`] and attach it with
/// [`ParserSettings::mta`](crate::ParserSettings::mta), or look up strings for an event with
/// [`MtaFile::rendering_info`].
#[derive(Debug, Clone, Default)]
pub struct MtaFile {
    /// `EVT` entries sorted by entry index, which is the event's 0-based position in the log.
    events: Vec<EventEntry>,
    /// Message of the first `EVT` entry with each `EventRecordID`.
    first_message: HashMap<u64, Option<u32>>,
    /// `MSG` strings by entry index.
    messages: HashMap<u32, Box<str>>,
    publishers: Vec<MtaPublisher>,
    /// Publisher index by exact name (the first publisher with a name wins).
    by_name: HashMap<String, usize>,
    default_publisher: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct EventEntry {
    index: u32,
    event_record_id: u64,
    message: Option<u32>,
}

/// The level, task, opcode and keyword names of one publisher in an [`MtaFile`].
///
/// These are the publisher's own tables. [`MtaFile::rendering_info`] also falls back to the
/// [`DEFAULT_PUBLISHER`] tables.
#[derive(Debug, Clone, Default)]
pub struct MtaPublisher {
    name: String,
    levels: HashMap<u32, Box<str>>,
    /// Keyed by the low 16 bits of the stored value: some Windows builds store `win:None` (0) as
    /// `0x10000`.
    tasks: HashMap<u16, Box<str>>,
    /// Keyed by `(opcode << 16) | task`, with task 0 for opcodes that are not tied to a task.
    opcodes: HashMap<u32, Box<str>>,
    /// Keyed by bit number (0 for `0x1`).
    keywords: HashMap<u32, Box<str>>,
}

impl MtaPublisher {
    /// The publisher (provider) name, as in `System/Provider/@Name`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The name of a level, such as `Information` for 4.
    pub fn level(&self, level: u8) -> Option<&str> {
        self.levels.get(&level.into()).map(AsRef::as_ref)
    }

    /// The name of a task.
    pub fn task(&self, task: u16) -> Option<&str> {
        self.tasks.get(&task).map(AsRef::as_ref)
    }

    /// The name of an opcode within a task. Opcodes that are not tied to a task are stored
    /// under task 0.
    pub fn opcode(&self, opcode: u8, task: u16) -> Option<&str> {
        let key = (u32::from(opcode) << 16) | u32::from(task);
        self.opcodes.get(&key).map(AsRef::as_ref)
    }

    /// The name of a keyword, by bit number (0 for the keyword mask `0x1`).
    pub fn keyword(&self, bit: u8) -> Option<&str> {
        self.keywords.get(&bit.into()).map(AsRef::as_ref)
    }
}

/// The `System` fields of an event that select its strings in an [`MtaFile`].
///
/// Windows treats a missing `System` element as 0, and so does [`MtaFile::rendering_info`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MtaEvent<'a> {
    /// `System/Provider/@Name`.
    pub provider: &'a str,
    /// The record number from the record header ([`EvtxRecord::event_record_id`]). In an
    /// exported log, records are numbered 1, 2, 3, ... in file order, which is the order of the
    /// `.MTA` file's messages.
    ///
    /// [`EvtxRecord::event_record_id`]: crate::EvtxRecord::event_record_id
    pub record_number: u64,
    /// `System/EventRecordID`: the record's ID in the log it was exported from.
    pub event_record_id: u64,
    /// `System/EventID/@Qualifiers`, which events from classic event sources have.
    pub qualifiers: u16,
    /// `System/Level`.
    pub level: u8,
    /// `System/Task`.
    pub task: u16,
    /// `System/Opcode`.
    pub opcode: u8,
    /// `System/Keywords`.
    pub keywords: u64,
}

/// An event's localized strings, as [`MtaFile::rendering_info`] finds them. A string the file
/// does not have is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderingInfo<'a> {
    pub message: Option<&'a str>,
    pub level: Option<&'a str>,
    pub task: Option<&'a str>,
    pub opcode: Option<&'a str>,
    /// Names of the keywords set in the event's keyword mask, lowest bit first.
    pub keywords: Vec<&'a str>,
}

impl MtaFile {
    /// Read and parse an `.MTA` file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, MtaError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| MtaError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(&bytes)
    }

    /// Parse the contents of an `.MTA` file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MtaError> {
        let header = |offset| read_u32_le(bytes, offset).ok_or(corrupt("header", 0));
        let magic = read_array::<8>(bytes, 0).ok_or(corrupt("header", 0))?;
        if &magic != MAGIC {
            return Err(MtaError::InvalidMagic { magic });
        }
        let (major, minor) = (header(8)?, header(12)?);
        if major != 1 {
            return Err(MtaError::UnsupportedVersion { major, minor });
        }

        let mut sections: [Option<Section<'_>>; 3] = [None; 3];
        let mut offset = HEADER_SIZE;
        for _ in 0..header(20)? {
            let at = offset as u64;
            let start = read_u64_le(bytes, offset).ok_or(corrupt("section descriptor", at))?;
            let size = read_u64_le(bytes, offset + 8).ok_or(corrupt("section descriptor", at))?;
            let name_len =
                read_u32_le(bytes, offset + 16).ok_or(corrupt("section descriptor", at))? as usize;
            let name = (offset + DESCRIPTOR_SIZE)
                .checked_add(name_len)
                .and_then(|end| bytes.get(offset + DESCRIPTOR_SIZE..end))
                .ok_or(corrupt("section name", at))?;
            offset += DESCRIPTOR_SIZE + name_len;
            let Some(slot) = SECTIONS.iter().position(|known| utf16_eq(name, known)) else {
                continue;
            };
            let data = usize::try_from(start)
                .ok()
                .zip(usize::try_from(size).ok())
                .and_then(|(start, size)| bytes.get(start..start.checked_add(size)?))
                .ok_or(corrupt("section", start))?;
            sections[slot].get_or_insert(Section {
                name: SECTIONS[slot],
                data,
                base: start,
            });
        }
        let [evt, msg, publ] = sections;
        let missing = |name| MtaError::MissingSection { name };
        let (evt, msg, publ) = (
            evt.ok_or(missing("EVT"))?,
            msg.ok_or(missing("MSG"))?,
            publ.ok_or(missing("PUB"))?,
        );

        let mut messages = HashMap::new();
        msg.for_each_record(|index, payload, at| {
            let len = read_u32_le(payload, 0).ok_or(corrupt("MSG entry", at))? as usize;
            let text = payload
                .get(4..)
                .and_then(|text| text.get(..len))
                .ok_or(corrupt("MSG entry", at))?;
            messages.entry(index).or_insert_with(|| decode_utf16(text));
            Ok(())
        })?;

        let mut events = Vec::new();
        evt.for_each_record(|index, payload, at| {
            let event_record_id = read_u64_le(payload, 0).ok_or(corrupt("EVT entry", at))?;
            let message = read_u64_le(payload, 8).ok_or(corrupt("EVT entry", at))?;
            events.push(EventEntry {
                index,
                event_record_id,
                message: u32::try_from(message).ok(),
            });
            Ok(())
        })?;
        // Stable, so the first entry with an index survives `dedup`.
        events.sort_by_key(|event| event.index);
        events.dedup_by_key(|event| event.index);
        let mut first_message = HashMap::with_capacity(events.len());
        for event in &events {
            first_message
                .entry(event.event_record_id)
                .or_insert(event.message);
        }

        let mut publishers = Vec::new();
        publ.for_each_record(|index, payload, at| {
            let publisher = parse_publisher(payload, &messages).ok_or(corrupt("PUB entry", at))?;
            publishers.push((index, publisher));
            Ok(())
        })?;
        publishers.sort_by_key(|(index, _)| *index);
        let publishers: Vec<_> = publishers.into_iter().map(|(_, p)| p).collect();
        let mut by_name = HashMap::with_capacity(publishers.len());
        for (index, publisher) in publishers.iter().enumerate() {
            by_name.entry(publisher.name.clone()).or_insert(index);
        }
        let default_publisher = by_name.get(DEFAULT_PUBLISHER).copied();

        Ok(MtaFile {
            events,
            first_message,
            messages,
            publishers,
            by_name,
            default_publisher,
        })
    }

    /// The publisher with this name, compared case-insensitively (ASCII).
    pub fn publisher(&self, name: &str) -> Option<&MtaPublisher> {
        match self.by_name.get(name) {
            Some(&index) => self.publishers.get(index),
            None => self
                .publishers
                .iter()
                .find(|publisher| publisher.name.eq_ignore_ascii_case(name)),
        }
    }

    /// All publishers in the file, including [`DEFAULT_PUBLISHER`].
    pub fn publishers(&self) -> impl Iterator<Item = &MtaPublisher> {
        self.publishers.iter()
    }

    /// The number of events the file has an entry for.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Look up an event's strings, or `None` if its publisher is not in the file (Windows then
    /// renders nothing).
    ///
    /// This follows what Windows returns when it renders an archived log, with two exceptions
    /// that only fill in strings Windows leaves empty (see `docs/mta.md`):
    /// - An opcode with no name within the event's task gets its name as an opcode of no task,
    ///   as Windows does for installed publishers (`Info` for opcode 0).
    /// - The message comes from the entry at the record's position in the log when that entry
    ///   has the event's `EventRecordID`. Windows takes the first entry with that ID, which is
    ///   another event's message when a log merges channels whose record IDs overlap.
    pub fn rendering_info(&self, event: &MtaEvent<'_>) -> Option<RenderingInfo<'_>> {
        let publisher = self.publisher(event.provider)?;
        let default = self.default_publisher.and_then(|i| self.publishers.get(i));
        // The first table that has the value wins, even with an empty name.
        let find = |lookup: &dyn Fn(&MtaPublisher) -> Option<&str>| {
            lookup(publisher).or_else(|| default.and_then(lookup))
        };
        let opcode = if event.qualifiers == 0 {
            find(&|p| p.opcode(event.opcode, event.task))
                .or_else(|| find(&|p| p.opcode(event.opcode, 0)))
        } else {
            // Windows names no opcode for events from classic event sources.
            None
        };
        let mut keywords = Vec::new();
        let mut mask = event.keywords;
        while mask != 0 {
            let bit = mask.trailing_zeros() as u8;
            mask &= mask - 1;
            keywords.extend(find(&|p| p.keyword(bit)).filter(|name| !name.is_empty()));
        }
        Some(RenderingInfo {
            message: self.message(event.record_number, event.event_record_id),
            level: non_empty(find(&|p| p.level(event.level))),
            task: non_empty(find(&|p| p.task(event.task))),
            opcode: non_empty(opcode),
            keywords,
        })
    }

    fn message(&self, record_number: u64, event_record_id: u64) -> Option<&str> {
        let at_position = record_number
            .checked_sub(1)
            .and_then(|position| u32::try_from(position).ok())
            .and_then(|position| {
                let found = self.events.binary_search_by_key(&position, |e| e.index);
                found.ok().map(|i| self.events[i])
            })
            .filter(|entry| entry.event_record_id == event_record_id);
        let message = match at_position {
            Some(entry) => entry.message,
            None => *self.first_message.get(&event_record_id)?,
        }?;
        non_empty(self.messages.get(&message).map(AsRef::as_ref))
    }
}

fn non_empty(name: Option<&str>) -> Option<&str> {
    name.filter(|name| !name.is_empty())
}

const SECTIONS: [&str; 3] = ["EVT", "MSG", "PUB"];

#[derive(Debug, Clone, Copy)]
struct Section<'b> {
    name: &'static str,
    data: &'b [u8],
    /// File offset of `data`, for errors.
    base: u64,
}

impl<'b> Section<'b> {
    /// Calls `f(entry index, payload, file offset)` for every record in the section's page chain.
    fn for_each_record(
        &self,
        mut f: impl FnMut(u32, &'b [u8], u64) -> Result<(), MtaError>,
    ) -> Result<(), MtaError> {
        let data = self.data;
        if data.is_empty() {
            return Ok(());
        }
        let at = |offset: usize| self.base.saturating_add(offset as u64);
        let mut page = 0;
        // Every page has its own header, so a longer chain revisits a page.
        for _ in 0..=data.len() / PAGE_HEADER_SIZE {
            let header = page_header(data, page).ok_or(corrupt("page", at(page)))?;
            // An empty page has 0 here, and no records.
            if read_u32_le(header, 0) != Some(0) {
                for slot in header[PAGE_RECORDS..].as_chunks::<8>().0 {
                    let offset = u64::from_le_bytes(*slot);
                    if offset == 0 {
                        continue;
                    }
                    let offset = usize::try_from(offset).unwrap_or(usize::MAX);
                    let (index, payload) =
                        record(data, offset).ok_or(corrupt("record", at(offset)))?;
                    f(index, payload, at(offset))?;
                }
            }
            let next = read_u64_le(header, PAGE_NEXT).expect("in page header");
            if next == read_u64_le(header, PAGE_SELF).expect("in page header") {
                return Ok(());
            }
            page = usize::try_from(next).unwrap_or(usize::MAX);
        }
        Err(MtaError::PageLoop { section: self.name })
    }
}

fn page_header(data: &[u8], offset: usize) -> Option<&[u8]> {
    data.get(offset..)?.get(..PAGE_HEADER_SIZE)
}

fn record(data: &[u8], offset: usize) -> Option<(u32, &[u8])> {
    let index = read_u32_le(data, offset)?;
    let size = read_u32_le(data, offset.checked_add(4)?)? as usize;
    let payload = data
        .get(offset.checked_add(RECORD_HEADER_SIZE)?..)?
        .get(..size)?;
    Some((index, payload))
}

/// A `PUB` entry: level, task, opcode and keyword tables (a u32 count, then (value, `MSG`
/// index) pairs of u32), then the name (u32 byte length, UTF-16LE). Table entries whose string
/// is missing are dropped.
fn parse_publisher(payload: &[u8], messages: &HashMap<u32, Box<str>>) -> Option<MtaPublisher> {
    let mut offset = 0;
    let mut next_table = || {
        let count = read_u32_le(payload, offset)? as usize;
        let entries = payload.get(offset + 4..)?.get(..count.checked_mul(8)?)?;
        offset += 4 + entries.len();
        let named = entries.as_chunks::<8>().0.iter().filter_map(|entry| {
            let value = read_u32_le(entry, 0).expect("8-byte entry");
            let message = read_u32_le(entry, 4).expect("8-byte entry");
            Some((value, messages.get(&message)?.clone()))
        });
        Some(named.collect::<Vec<_>>())
    };
    let levels = collect_first(next_table()?);
    let tasks = collect_first(
        next_table()?
            .into_iter()
            .map(|(value, name)| (value as u16, name)),
    );
    let opcodes = collect_first(next_table()?);
    let keywords = collect_first(next_table()?);
    let name_len = read_u32_le(payload, offset)? as usize;
    let name = payload.get(offset + 4..)?.get(..name_len)?;
    Some(MtaPublisher {
        name: decode_utf16(name).into(),
        levels,
        tasks,
        opcodes,
        keywords,
    })
}

/// Collects into a map, keeping the first value for a repeated key.
fn collect_first<K: std::hash::Hash + Eq, V>(
    entries: impl IntoIterator<Item = (K, V)>,
) -> HashMap<K, V> {
    let mut map = HashMap::new();
    for (key, value) in entries {
        map.entry(key).or_insert(value);
    }
    map
}

/// Decodes UTF-16LE text up to the first NUL, replacing unpaired surrogates.
fn decode_utf16(bytes: &[u8]) -> Box<str> {
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|unit| u16::from_le_bytes(*unit))
        .take_while(|&unit| unit != 0);
    let mut out = String::with_capacity(bytes.len() / 2);
    out.extend(char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)));
    out.into_boxed_str()
}

fn utf16_eq(bytes: &[u8], ascii: &str) -> bool {
    let units = bytes.as_chunks::<2>().0;
    // Ignore a terminating NUL, if any.
    let units = match units {
        [rest @ .., [0, 0]] => rest,
        _ => units,
    };
    units.len() == ascii.len()
        && units
            .iter()
            .zip(ascii.bytes())
            .all(|(unit, byte)| *unit == [byte, 0])
}

fn corrupt(what: &'static str, offset: u64) -> MtaError {
    MtaError::Corrupt { what, offset }
}

/// Appends a `RenderingInfo` element with the record's strings from `mta` to its `Event`
/// element. Records whose publisher is not in `mta`, and records that already have a
/// `RenderingInfo` (forwarded events), are left as they are.
pub(crate) fn add_rendering_info<'a>(
    tree: &mut IrTree<'a>,
    record_number: u64,
    mta: &'a MtaFile,
    bump: &'a Bump,
) {
    // A record's tree has no substitutions left.
    let Some(info) = SystemFields::of(tree)
        .and_then(|fields| fields.rendering_info(mta, record_number, |_| Value::None))
    else {
        return;
    };
    let root = tree.root();
    let arena = tree.arena_mut();
    let rendering_info = rendering_info_element(&info, arena, bump);
    arena
        .get_mut(root)
        .expect("root is in the arena")
        .push_child(Node::Element(rendering_info));
}

/// Builds the `RenderingInfo` element for `info` in `arena`.
pub(crate) fn rendering_info_element<'a>(
    info: &RenderingInfo<'a>,
    arena: &mut IrArena<'a>,
    bump: &'a Bump,
) -> ElementId {
    let mut text_element = |name: &'static str, text: &'a str| {
        let mut element = Element::new_in(Name::new(name), bump);
        element.push_child(Node::Text(Text::utf8(text)));
        Node::Element(arena.new_node(element))
    };
    let mut rendering_info = Element::new_in(Name::new("RenderingInfo"), bump);
    let fields = [
        ("Message", info.message),
        ("Level", info.level),
        ("Task", info.task),
        ("Opcode", info.opcode),
    ];
    for (name, text) in fields {
        if let Some(text) = text {
            rendering_info.push_child(text_element(name, text));
        }
    }
    if !info.keywords.is_empty() {
        let mut keywords = Element::new_in(Name::new("Keywords"), bump);
        for keyword in &info.keywords {
            keywords.push_child(text_element("Keyword", keyword));
        }
        rendering_info.push_child(Node::Element(arena.new_node(keywords)));
    }
    arena.new_node(rendering_info)
}

/// Where a tree keeps the `System` fields that select an event's strings in an [`MtaFile`].
///
/// The tree is a record's, or a template definition's, whose fields can be substitutions. The
/// compiled renderer then reads them from each record's substitution values, without building
/// the record's tree.
#[derive(Debug, Clone, Default)]
pub(crate) struct SystemFields {
    /// Substitutions among the children of `Event`. Usually one holds the event data, but a
    /// `System` or `RenderingInfo` there changes what the fields are.
    event_slots: Vec<u16>,
    /// Substitutions among the children of `System`, where an element could be a field.
    system_slots: Vec<u16>,
    /// `None` when the event gets no `RenderingInfo`: the root is not `Event`, it has no
    /// `System`, or it has a `RenderingInfo` already.
    sources: Option<Sources>,
}

#[derive(Debug, Clone, Default)]
struct Sources {
    provider: Source<Box<str>>,
    qualifiers: Source<Option<u64>>,
    level: Source<Option<u64>>,
    task: Source<Option<u64>>,
    opcode: Source<Option<u64>>,
    keywords: Source<Option<u64>>,
    event_record_id: Source<Option<u64>>,
}

/// A field's value, read from literal nodes, or the substitution that holds it.
#[derive(Debug, Clone)]
enum Source<T> {
    Literal(T),
    Substitution(u16),
}

/// A missing field reads as no nodes.
impl<T: Default> Default for Source<T> {
    fn default() -> Self {
        Source::Literal(T::default())
    }
}

/// A substitution value, as far as [`SystemFields`] reads it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Value<'v> {
    /// No value, or one that is neither a number nor text (a GUID, say).
    None,
    /// Embedded BinXML: the root element name of a template instance, or `None` for another
    /// fragment.
    Element(Option<&'v [u8]>),
    /// An integer that is not negative.
    Number(u64),
    Utf16(&'v [u8]),
    Text(&'v str),
}

impl<'v> Value<'v> {
    fn number(self) -> Option<u64> {
        match self {
            Value::Number(number) => Some(number),
            Value::Utf16(_) | Value::Text(_) => parse_number(&self.text()),
            Value::None | Value::Element(_) => None,
        }
    }

    fn text(self) -> Cow<'v, str> {
        match self {
            Value::Utf16(bytes) => Cow::Owned(decode_utf16(bytes).into()),
            Value::Text(text) => Cow::Borrowed(text),
            Value::None | Value::Element(_) | Value::Number(_) => Cow::Borrowed(""),
        }
    }
}

impl SystemFields {
    /// `None` for a template whose fields only a record's tree shows, such as a field that
    /// mixes text and substitutions.
    pub(crate) fn of(tree: &IrTree<'_>) -> Option<Self> {
        let mut fields = SystemFields::default();
        let root = tree.root_element();
        if root.name.as_str() != "Event" {
            return Some(fields);
        }
        let mut system = None;
        for node in &root.children {
            match node {
                Node::Element(id) => match tree.element(*id).name.as_str() {
                    "System" => system = system.or(Some(*id)),
                    "RenderingInfo" => return Some(SystemFields::default()),
                    _ => {}
                },
                Node::Placeholder(placeholder) => fields.event_slots.push(placeholder.id),
                _ => {}
            }
        }
        let Some(system) = system else {
            return Some(fields);
        };
        let mut sources = Sources::default();
        for node in &tree.element(system).children {
            let element = match node {
                Node::Element(id) => tree.element(*id),
                Node::Placeholder(placeholder) => {
                    fields.system_slots.push(placeholder.id);
                    continue;
                }
                _ => continue,
            };
            let content = || source(&element.children, number_value);
            match element.name.as_str() {
                "Provider" => {
                    sources.provider =
                        source(attribute(element, "Name")?, |nodes| text(nodes).into())?
                }
                "EventID" => {
                    sources.qualifiers = source(attribute(element, "Qualifiers")?, number_value)?
                }
                "Level" => sources.level = content()?,
                "Task" => sources.task = content()?,
                "Opcode" => sources.opcode = content()?,
                "Keywords" => sources.keywords = content()?,
                "EventRecordID" => sources.event_record_id = content()?,
                _ => {}
            }
        }
        fields.sources = Some(sources);
        Some(fields)
    }

    /// Whether a record has elements in a substitution where they could change its fields.
    /// `value` gives the record's substitution values.
    pub(crate) fn needs_tree<'v>(&self, value: impl Fn(u16) -> Value<'v>) -> bool {
        let changes_event = |id| match value(id) {
            Value::Element(Some(root)) => root == b"System" || root == b"RenderingInfo",
            Value::Element(None) => true,
            _ => false,
        };
        let is_element = |id| matches!(value(id), Value::Element(_));
        self.event_slots.iter().any(|&id| changes_event(id))
            || self.system_slots.iter().any(|&id| is_element(id))
    }

    /// A record's strings, or `None` when it gets no `RenderingInfo`. `value` gives the
    /// record's substitution values.
    pub(crate) fn rendering_info<'m, 'v>(
        &self,
        mta: &'m MtaFile,
        record_number: u64,
        value: impl Fn(u16) -> Value<'v>,
    ) -> Option<RenderingInfo<'m>> {
        let sources = self.sources.as_ref()?;
        let number = |source: &Source<Option<u64>>| match *source {
            Source::Literal(number) => number,
            Source::Substitution(id) => value(id).number(),
        };
        let provider = match &sources.provider {
            Source::Literal(name) => Cow::Borrowed(&**name),
            Source::Substitution(id) => value(*id).text(),
        };
        mta.rendering_info(&MtaEvent {
            provider: &provider,
            record_number,
            event_record_id: number(&sources.event_record_id).unwrap_or(0),
            qualifiers: narrow(number(&sources.qualifiers)),
            level: narrow(number(&sources.level)),
            task: narrow(number(&sources.task)),
            opcode: narrow(number(&sources.opcode)),
            keywords: number(&sources.keywords).unwrap_or(0),
        })
    }
}

/// A field held by `nodes`: one substitution, or literal nodes that `literal` reads. `None`
/// when substitutions are mixed with other nodes.
fn source<T>(nodes: &[Node<'_>], literal: impl FnOnce(&[Node<'_>]) -> T) -> Option<Source<T>> {
    match nodes {
        [Node::Placeholder(placeholder)] => Some(Source::Substitution(placeholder.id)),
        _ if has_substitution(nodes) => None,
        _ => Some(Source::Literal(literal(nodes))),
    }
}

fn has_substitution(nodes: &[Node<'_>]) -> bool {
    nodes
        .iter()
        .any(|node| matches!(node, Node::Placeholder(_)))
}

/// The value of the first attribute called `name` (no nodes if there is none). `None` when the
/// name repeats and has a substitution: a record's tree leaves out attributes whose
/// substitutions are empty, so which one comes first depends on the record.
fn attribute<'e, 'a>(element: &'e Element<'a>, name: &str) -> Option<&'e [Node<'a>]> {
    let mut attrs = element
        .attrs
        .iter()
        .filter(|attr| attr.name.as_str() == name);
    let Some(first) = attrs.next() else {
        return Some(&[]);
    };
    if has_substitution(&first.value) && attrs.next().is_some() {
        return None;
    }
    Some(&first.value)
}

/// A missing or out-of-range value counts as 0.
fn narrow<T: TryFrom<u64> + Default>(value: Option<u64>) -> T {
    value
        .and_then(|value| T::try_from(value).ok())
        .unwrap_or_default()
}

/// The value of a number-valued element or attribute: a single typed value, or text in decimal
/// or `0x`-prefixed hex.
fn number_value(nodes: &[Node<'_>]) -> Option<u64> {
    if let [Node::Value(value)] = nodes
        && let Some(number) = value_number(value)
    {
        return Some(number);
    }
    parse_number(&text(nodes))
}

fn parse_number(text: &str) -> Option<u64> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

fn value_number(value: &BinXmlValue<'_>) -> Option<u64> {
    match *value {
        BinXmlValue::UInt8Type(v) => Some(v.into()),
        BinXmlValue::UInt16Type(v) => Some(v.into()),
        BinXmlValue::UInt32Type(v) | BinXmlValue::HexInt32Type(v) => Some(v.into()),
        BinXmlValue::UInt64Type(v) | BinXmlValue::HexInt64Type(v) => Some(v),
        BinXmlValue::Int8Type(v) => u64::try_from(v).ok(),
        BinXmlValue::Int16Type(v) => u64::try_from(v).ok(),
        BinXmlValue::Int32Type(v) => u64::try_from(v).ok(),
        BinXmlValue::Int64Type(v) => u64::try_from(v).ok(),
        BinXmlValue::SizeTType(v) => u64::try_from(v).ok(),
        _ => None,
    }
}

/// The text of string-valued nodes, concatenated.
fn text(nodes: &[Node<'_>]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(Text::Utf8(s)) | Node::Value(BinXmlValue::AnsiStringType(s)) => {
                out.push_str(s)
            }
            Node::Text(Text::Utf16(s)) | Node::Value(BinXmlValue::StringType(s)) => {
                out.push_str(&decode_utf16(s.as_bytes()))
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// A section with `records` laid out in pages of 100, like Windows writes them. No records
    /// gives one empty page.
    fn section(records: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let pages: Vec<_> = records.chunks(100).collect();
        let pages = if pages.is_empty() {
            vec![records]
        } else {
            pages
        };
        let mut out = Vec::new();
        for (number, page) in pages.iter().enumerate() {
            let start = out.len();
            let mut header = vec![0; PAGE_HEADER_SIZE];
            if !page.is_empty() {
                header[..4].copy_from_slice(&u32::MAX.to_le_bytes());
                header[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
            }
            header[64..72].copy_from_slice(&(number as u64).to_le_bytes());
            let mut body = Vec::new();
            for (slot, (index, payload)) in page.iter().enumerate() {
                let offset = (start + PAGE_HEADER_SIZE + body.len()) as u64;
                header[PAGE_RECORDS + slot * 8..][..8].copy_from_slice(&offset.to_le_bytes());
                body.extend(index.to_le_bytes());
                body.extend((payload.len() as u32).to_le_bytes());
                body.extend(payload);
            }
            let next = if number + 1 == pages.len() {
                start
            } else {
                start + PAGE_HEADER_SIZE + body.len()
            };
            header[PAGE_NEXT..][..8].copy_from_slice(&(next as u64).to_le_bytes());
            header[PAGE_SELF..][..8].copy_from_slice(&(start as u64).to_le_bytes());
            out.extend(header);
            out.extend(body);
        }
        out
    }

    fn file(sections: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let table_size: usize = sections
            .iter()
            .map(|(name, _)| DESCRIPTOR_SIZE + utf16(name).len())
            .sum();
        let mut out = MAGIC.to_vec();
        for value in [1, 1, table_size as u32, sections.len() as u32] {
            out.extend(u32::to_le_bytes(value));
        }
        let mut offset = HEADER_SIZE + table_size;
        for (name, data) in sections {
            out.extend((offset as u64).to_le_bytes());
            out.extend((data.len() as u64).to_le_bytes());
            out.extend((utf16(name).len() as u32).to_le_bytes());
            out.extend(utf16(name));
            offset += data.len();
        }
        for (_, data) in sections {
            out.extend(data);
        }
        out
    }

    fn message(text: &str) -> Vec<u8> {
        let text = if text.is_empty() {
            Vec::new()
        } else {
            utf16(&format!("{text}\0"))
        };
        let mut out = (text.len() as u32).to_le_bytes().to_vec();
        out.extend(text);
        out
    }

    fn event(event_record_id: u64, message: u64) -> Vec<u8> {
        [event_record_id, message]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// Tables of (value, message index): levels, tasks, opcodes, keywords.
    fn publisher(name: &str, tables: [&[(u32, u32)]; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        for table in tables {
            out.extend((table.len() as u32).to_le_bytes());
            for (value, message) in table {
                out.extend(value.to_le_bytes());
                out.extend(message.to_le_bytes());
            }
        }
        out.extend((utf16(name).len() as u32).to_le_bytes());
        out.extend(utf16(name));
        out
    }

    /// Messages: 0 "Log Always", 1 "Information", 2 "None", 3 "Info", 4 "Start", 5 "Classic",
    /// 6 "Provider info", 7 "Connect", 8 "Handshake", 9 "Network", 10 "", 11 "first", 12
    /// "second", 13 "Global".
    fn sample() -> MtaFile {
        let strings = [
            "Log Always",
            "Information",
            "None",
            "Info",
            "Start",
            "Classic",
            "Provider info",
            "Connect",
            "Handshake",
            "Network",
            "",
            "first",
            "second",
            "Global",
        ];
        let messages: Vec<_> = (0..)
            .zip(strings)
            .map(|(index, text)| (index, message(text)))
            .collect();
        let events = vec![(0, event(10, 11)), (1, event(10, 12)), (2, event(11, 10))];
        let publishers = vec![
            (
                0,
                publisher(
                    DEFAULT_PUBLISHER,
                    [
                        &[(0, 0), (4, 1)],
                        &[(0x10000, 2)],
                        &[(0, 3), (1 << 16, 4)],
                        &[(55, 5)],
                    ],
                ),
            ),
            (
                1,
                publisher(
                    "Provider",
                    [
                        &[(4, 6), (5, 10)],
                        &[(1, 7)],
                        &[((10 << 16) | 1, 8), (11 << 16, 13)],
                        &[(0, 9), (55, 10)],
                    ],
                ),
            ),
        ];
        let bytes = file(&[
            ("EVT", section(&events)),
            ("MSG", section(&messages)),
            ("PUB", section(&publishers)),
        ]);
        MtaFile::from_bytes(&bytes).unwrap()
    }

    fn info<'m>(mta: &'m MtaFile, event: MtaEvent<'_>) -> RenderingInfo<'m> {
        mta.rendering_info(&MtaEvent {
            provider: "Provider",
            ..event
        })
        .unwrap()
    }

    #[test]
    fn publisher_tables_then_default() {
        let mta = sample();
        let event = MtaEvent {
            level: 4,
            ..MtaEvent::default()
        };
        assert_eq!(info(&mta, event).level, Some("Provider info"));
        let event = MtaEvent { level: 0, ..event };
        assert_eq!(info(&mta, event).level, Some("Log Always"));
        // An empty name in the publisher's table is not replaced by the default one.
        let event = MtaEvent { level: 5, ..event };
        assert_eq!(info(&mta, event).level, None);
        // The default publisher stores `None` as 0x10000 here; tasks match on 16 bits.
        assert_eq!(info(&mta, MtaEvent::default()).task, Some("None"));
        assert_eq!(
            info(&mta, MtaEvent { task: 1, ..event }).task,
            Some("Connect")
        );
        assert!(mta.rendering_info(&MtaEvent::default()).is_none());
        assert!(
            mta.rendering_info(&MtaEvent {
                provider: "PROVIDER",
                ..MtaEvent::default()
            })
            .is_some()
        );
    }

    #[test]
    fn opcodes() {
        let mta = sample();
        let opcode = |opcode, task, qualifiers| {
            let event = MtaEvent {
                opcode,
                task,
                qualifiers,
                ..MtaEvent::default()
            };
            info(&mta, event).opcode
        };
        assert_eq!(opcode(10, 1, 0), Some("Handshake"));
        // Not in task 1: the opcode's name outside any task, from the publisher or default.
        assert_eq!(opcode(11, 1, 0), Some("Global"));
        assert_eq!(opcode(1, 1, 0), Some("Start"));
        assert_eq!(opcode(0, 0, 0), Some("Info"));
        assert_eq!(opcode(10, 2, 0), None);
        // Classic events have no opcode name.
        assert_eq!(opcode(0, 0, 16384), None);
    }

    #[test]
    fn keywords_lowest_bit_first() {
        let mta = sample();
        let event = MtaEvent {
            keywords: 0x8080_0000_0000_0003,
            ..MtaEvent::default()
        };
        // Bit 1 and bit 63 have no name; the publisher's empty name for bit 55 is skipped.
        assert_eq!(info(&mta, event).keywords, ["Network"]);
        let event = MtaEvent {
            keywords: 1 << 55,
            provider: "other",
            ..event
        };
        assert!(mta.rendering_info(&event).is_none());
    }

    #[test]
    fn messages_by_position_then_record_id() {
        let mta = sample();
        let message = |record_number, event_record_id| {
            let event = MtaEvent {
                record_number,
                event_record_id,
                ..MtaEvent::default()
            };
            info(&mta, event).message
        };
        assert_eq!(message(1, 10), Some("first"));
        // Two events with record ID 10, as in a log that merges channels.
        assert_eq!(message(2, 10), Some("second"));
        // No entry at this position for record ID 10: the first entry with that ID.
        assert_eq!(message(7, 10), Some("first"));
        assert_eq!(message(3, 10), Some("first"));
        assert_eq!(message(0, 10), Some("first"));
        // The entry's message is empty.
        assert_eq!(message(3, 11), None);
        assert_eq!(message(1, 12), None);
    }

    #[test]
    fn many_pages_and_empty_pages() {
        let messages: Vec<_> = (0..250)
            .map(|index| (index, message(&format!("message {index}"))))
            .collect();
        let events: Vec<_> = (0..250)
            .map(|index| (index, event(1000 + u64::from(index), u64::from(index))))
            .collect();
        let bytes = file(&[
            ("EVT", section(&events)),
            ("MSG", section(&messages)),
            ("PUB", section(&[(0, publisher("p", [&[]; 4]))])),
        ]);
        let mta = MtaFile::from_bytes(&bytes).unwrap();
        assert_eq!(mta.event_count(), 250);
        let event = MtaEvent {
            provider: "p",
            record_number: 201,
            event_record_id: 1200,
            ..MtaEvent::default()
        };
        assert_eq!(
            mta.rendering_info(&event).unwrap().message,
            Some("message 200")
        );

        let bytes = file(&[
            ("EVT", section(&[])),
            ("MSG", section(&[])),
            ("PUB", section(&[])),
            ("XYZ", vec![1, 2, 3]),
        ]);
        let mta = MtaFile::from_bytes(&bytes).unwrap();
        assert_eq!(mta.event_count(), 0);
        assert_eq!(mta.publishers().count(), 0);
    }

    #[test]
    fn rejects_malformed_files() {
        let sections = || {
            vec![
                ("EVT", section(&[(0, event(1, 0))])),
                ("MSG", section(&[(0, message("a"))])),
                ("PUB", section(&[])),
            ]
        };
        let good = file(&sections());
        assert!(MtaFile::from_bytes(&good).is_ok());

        let mut bad = good.clone();
        bad[0] = b'X';
        assert!(matches!(
            MtaFile::from_bytes(&bad),
            Err(MtaError::InvalidMagic { .. })
        ));
        let mut bad = good.clone();
        bad[8] = 2;
        assert!(matches!(
            MtaFile::from_bytes(&bad),
            Err(MtaError::UnsupportedVersion { major: 2, minor: 1 })
        ));
        let mut without_pub = sections();
        without_pub.pop();
        assert!(matches!(
            MtaFile::from_bytes(&file(&without_pub)),
            Err(MtaError::MissingSection { name: "PUB" })
        ));
        assert!(matches!(
            MtaFile::from_bytes(&good[..good.len() - 1]),
            Err(MtaError::Corrupt {
                what: "section",
                ..
            })
        ));

        // A page whose next page is itself, under a different offset.
        let mut looping = sections();
        let evt = &mut looping[0].1;
        evt[PAGE_NEXT..][..8].copy_from_slice(&0u64.to_le_bytes());
        evt[PAGE_SELF..][..8].copy_from_slice(&1u64.to_le_bytes());
        assert!(matches!(
            MtaFile::from_bytes(&file(&looping)),
            Err(MtaError::PageLoop { section: "EVT" })
        ));

        let mut outside = sections();
        let msg = &mut outside[1].1;
        msg[PAGE_RECORDS..][..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(
            MtaFile::from_bytes(&file(&outside)),
            Err(MtaError::Corrupt { what: "record", .. })
        ));

        let mut long_string = sections();
        let msg = &mut long_string[1].1;
        msg[PAGE_HEADER_SIZE + RECORD_HEADER_SIZE..][..4].copy_from_slice(&100u32.to_le_bytes());
        assert!(matches!(
            MtaFile::from_bytes(&file(&long_string)),
            Err(MtaError::Corrupt {
                what: "MSG entry",
                ..
            })
        ));
    }

    #[test]
    fn damaged_files_do_not_panic() {
        let bytes = include_bytes!("../samples/mta/LocaleMetaData/classic_1033.MTA");
        assert!(MtaFile::from_bytes(bytes).is_ok());
        for len in 0..bytes.len() {
            let _ = MtaFile::from_bytes(&bytes[..len]);
        }
        let mut damaged = bytes.to_vec();
        for i in 0..damaged.len() {
            for value in [0x00, 0x7f, 0xff] {
                let original = std::mem::replace(&mut damaged[i], value);
                let _ = MtaFile::from_bytes(&damaged);
                damaged[i] = original;
            }
        }
    }

    /// An archive with strings for every record in `records` (provider name and
    /// `EventRecordID`, in file order): a message each (every fifth one empty), and level, task,
    /// opcode and keyword names for each provider and the default publisher.
    fn archive_for(records: &[(String, u64)]) -> MtaFile {
        let mut messages = Vec::new();
        let mut string = |text: String| {
            messages.push((messages.len() as u32, message(&text)));
            messages.len() as u32 - 1
        };
        let mut tables = |owner: &str| -> [Vec<(u32, u32)>; 4] {
            let mut name = |kind: &str, value: u32| string(format!("{owner} {kind} {value} <&>"));
            [
                (0..=5).map(|level| (level, name("level", level))).collect(),
                (0..=40).map(|task| (task, name("task", task))).collect(),
                (0..=12)
                    .map(|opcode| (opcode << 16, name("opcode", opcode)))
                    .collect(),
                (0..64)
                    .step_by(3)
                    .map(|bit| (bit, name("keyword", bit)))
                    .collect(),
            ]
        };
        let mut providers: Vec<&str> = records.iter().map(|(provider, _)| &**provider).collect();
        providers.sort();
        providers.dedup();
        let publishers: Vec<_> = std::iter::once(DEFAULT_PUBLISHER)
            .chain(providers)
            .enumerate()
            .map(|(index, name)| {
                let [levels, tasks, opcodes, keywords] = tables(name);
                let tables = [&levels[..], &tasks, &opcodes, &keywords];
                (index as u32, publisher(name, tables))
            })
            .collect();
        let events: Vec<_> = (0..)
            .zip(records)
            .map(|(index, (_, event_record_id))| {
                let text = if index % 5 == 4 {
                    String::new()
                } else {
                    format!("Event {event_record_id}: <a & \"b\"> 'c'\r\n\t\u{e9}\u{1F600}")
                };
                (index, event(*event_record_id, string(text).into()))
            })
            .collect();
        let bytes = file(&[
            ("EVT", section(&events)),
            ("MSG", section(&messages)),
            ("PUB", section(&publishers)),
        ]);
        MtaFile::from_bytes(&bytes).unwrap()
    }

    /// Renders the first records of `path`, with an archive that has strings for them, through
    /// the chunk renderer, which adds `RenderingInfo` to records it renders from compiled
    /// templates, and through the record API, which adds it to each record's tree. Returns how
    /// many renderings had one.
    fn compare_renderers(path: &Path) -> usize {
        use crate::{EvtxParser, ParserSettings, RenderedChunkItem};
        use std::sync::Arc;

        const RECORDS: usize = 2000;
        let records: Vec<(String, u64)> = EvtxParser::from_path(path)
            .unwrap()
            .records_json_value()
            .filter_map(Result::ok)
            .take(RECORDS)
            .filter_map(|record| {
                let system = &record.data["Event"]["System"];
                let provider = system["Provider"]["#attributes"]["Name"].as_str()?;
                Some((provider.to_string(), system["EventRecordID"].as_u64()?))
            })
            .collect();
        let mta = Arc::new(archive_for(&records));
        let mut with_info = 0;
        for (json, indent) in [(true, false), (false, true), (false, false)] {
            let settings = ParserSettings::new()
                .num_threads(1)
                .indent(indent)
                .mta(Some(Arc::clone(&mta)));
            let mut parser = EvtxParser::from_path(path)
                .unwrap()
                .with_configuration(settings);
            let tree: Vec<(u64, String)> = if json {
                let records = parser.records_json().filter_map(Result::ok);
                records
                    .map(|r| (r.event_record_id, r.data))
                    .take(RECORDS)
                    .collect()
            } else {
                let records = parser.records().filter_map(Result::ok);
                records
                    .map(|r| (r.event_record_id, r.data))
                    .take(RECORDS)
                    .collect()
            };
            let chunks: Box<dyn Iterator<Item = _>> = if json {
                Box::new(parser.chunks_json_bytes(false))
            } else {
                Box::new(parser.chunks_xml_bytes(false))
            };
            let mut compiled = Vec::new();
            for chunk in chunks {
                let mut start = 0;
                for item in chunk.items {
                    if let RenderedChunkItem::Record {
                        event_record_id,
                        end,
                    } = item
                    {
                        let data = std::str::from_utf8(&chunk.data[start..end - 1]).unwrap();
                        compiled.push((event_record_id, data.to_string()));
                        start = end;
                    }
                }
                if compiled.len() >= RECORDS {
                    break;
                }
            }
            compiled.truncate(RECORDS);
            assert_eq!(compiled.len(), tree.len(), "{}", path.display());
            for (compiled, tree) in compiled.iter().zip(&tree) {
                assert_eq!(compiled, tree, "{} (json: {json})", path.display());
            }
            with_info += tree
                .iter()
                .filter(|(_, data)| data.contains("RenderingInfo"))
                .count();
        }
        with_info
    }

    #[test]
    fn compiled_and_tree_rendering_agree() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
        let mut paths: Vec<_> = [samples.clone(), samples.join("mta")]
            .iter()
            .flat_map(|dir| std::fs::read_dir(dir).unwrap())
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "evtx"))
            .collect();
        paths.sort();
        let with_info: Vec<_> = std::thread::scope(|scope| {
            let checks: Vec<_> = (paths.iter())
                .map(|path| scope.spawn(move || (path, compare_renderers(path))))
                .collect();
            checks
                .into_iter()
                .map(|check| check.join().unwrap())
                .collect()
        });
        for (path, count) in &with_info {
            eprintln!("{count:>8} {}", path.display());
        }
        assert!(with_info.iter().map(|(_, count)| count).sum::<usize>() > 0);
    }
}
