#![allow(dead_code)]
use std::path::PathBuf;

use std::sync::Mutex;
use std::sync::Once;

use evtx::{EvtxParser, ParserSettings};

static LOGGER_INIT: Once = Once::new();

// CLI tests can behave flakily when multiple `evtx_dump` subprocesses run concurrently
// (especially when using pty-based interaction). Serialize all CLI-spawning tests.
pub static CLI_TEST_LOCK: Mutex<()> = Mutex::new(());

// Rust runs the tests concurrently, so unless we synchronize logging access
// it will crash when attempting to run `cargo test` with some logging facilities.
#[cfg(test)]
pub fn ensure_env_logger_initialized() {
    use std::io::Write;

    LOGGER_INIT.call_once(|| {
        let mut builder = env_logger::Builder::from_default_env();
        builder
            .format(|buf, record| writeln!(buf, "[{}] - {}", record.level(), record.args()))
            .init();
    });
}

pub fn samples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .canonicalize()
        .unwrap()
}

pub fn regular_sample() -> PathBuf {
    samples_dir().join("security.evtx")
}

pub fn sample_with_irregular_values() -> PathBuf {
    samples_dir().join("sample-with-irregular-bool-values.evtx")
}

pub fn sample_with_a_bad_checksum() -> PathBuf {
    samples_dir()
        .join("2-vss_0-Microsoft-Windows-RemoteDesktopServices-RdpCoreTS%4Operational.evtx")
}

pub fn sample_with_a_bad_checksum_2() -> PathBuf {
    samples_dir().join(
        "2-vss_0-Microsoft-Windows-TerminalServices-RemoteConnectionManager%4Operational.evtx",
    )
}

pub fn sample_with_a_chunk_past_zeroes() -> PathBuf {
    samples_dir().join("2-vss_7-System.evtx")
}

pub fn sample_with_a_bad_chunk_magic() -> PathBuf {
    samples_dir().join("sample_with_a_bad_chunk_magic.evtx")
}

pub fn sample_binxml_with_incomplete_sid() -> PathBuf {
    samples_dir().join("Microsoft-Windows-HelloForBusiness%4Operational.evtx")
}

pub fn sample_binxml_with_incomplete_template() -> PathBuf {
    samples_dir().join("Microsoft-Windows-LanguagePackSetup%4Operational.evtx")
}

pub fn sample_with_multiple_xml_fragments() -> PathBuf {
    samples_dir()
        .join("E_Windows_system32_winevt_logs_Microsoft-Windows-Shell-Core%4Operational.evtx")
}

pub fn sample_with_binxml_as_substitution_tokens_and_pi_target() -> PathBuf {
    samples_dir().join("E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx")
}

pub fn sample_issue_65() -> PathBuf {
    samples_dir().join(
        "E_ShadowCopy6_windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx",
    )
}

pub fn sample_with_dependency_id_edge_case() -> PathBuf {
    samples_dir().join("Archive-ForwardedEvents-test.evtx")
}

pub fn sample_with_no_crc32() -> PathBuf {
    samples_dir().join("Application_no_crc32.evtx")
}

pub fn sample_with_invalid_flags_in_header() -> PathBuf {
    samples_dir().join("post-Security.evtx")
}

/// The EVTX layout (the crate keeps its own copies private).
pub const EVTX_FILE_HEADER_SIZE: usize = 4096;
pub const EVTX_CHUNK_SIZE: usize = 65536;
pub const EVTX_CHUNK_HEADER_SIZE: usize = 512;
pub const EVTX_RECORD_HEADER_SIZE: usize = 24;
pub const EVTX_RECORD_MAGIC: &[u8; 4] = b"**\0\0";
/// A template definition's header: next template, GUID, data size.
pub const TEMPLATE_DEFINITION_HEADER_SIZE: usize = 24;

/// Record `id` of `evtx` as XML and JSON, from the records API. evtx_dump's chunk renderer
/// must give the same bytes.
pub fn render_record(evtx: &[u8], id: u64) -> (String, String) {
    let parser = || {
        EvtxParser::from_buffer(evtx.to_vec())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };
    let rendered = |chunks: Vec<evtx::RenderedChunk>| {
        let (_, record) = chunks
            .iter()
            .flat_map(|chunk| chunk.records())
            .find(|&(record_id, _)| record_id == id)
            .expect("the chunk renderer to render the record");
        String::from_utf8(record.to_vec()).unwrap()
    };

    let xml = parser()
        .records()
        .map(Result::unwrap)
        .find(|record| record.event_record_id == id)
        .expect("the record to render")
        .data;
    assert_eq!(rendered(parser().chunks_xml_bytes(false).collect()), xml);
    let json = parser()
        .records_json()
        .map(Result::unwrap)
        .find(|record| record.event_record_id == id)
        .expect("the record to render")
        .data;
    assert_eq!(rendered(parser().chunks_json_bytes(false).collect()), json);
    (xml, json)
}

/// BinXML for synthetic records, written at a known chunk offset: inline names and inline
/// template definitions point at their own offset.
pub struct BinXml {
    bytes: Vec<u8>,
    base: u32,
    /// The template definition `template_instance` wrote last, here or in an enclosing or
    /// earlier stream, for `template_ref`.
    definition: Option<u32>,
    /// The name `open` or `attribute` wrote last, for `reopen` and `reattribute`.
    name: Option<u32>,
}

/// A template value: its type, and what writes its bytes at the offset they land at.
pub type Value<'v> = (u8, Box<dyn FnOnce(&mut BinXml) + 'v>);

/// A template value of type `value_type` that `write` writes.
pub fn value<'v>(value_type: u8, write: impl FnOnce(&mut BinXml) + 'v) -> Value<'v> {
    (value_type, Box::new(write))
}

impl BinXml {
    fn at(base: u32) -> Self {
        BinXml {
            bytes: Vec::new(),
            base,
            definition: None,
            name: None,
        }
    }

    fn pos(&self) -> u32 {
        self.base + self.bytes.len() as u32
    }

    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    fn u8(&mut self, v: u8) -> &mut Self {
        self.raw(&[v])
    }

    fn u16(&mut self, v: u16) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }

    fn u32(&mut self, v: u32) -> &mut Self {
        self.raw(&v.to_le_bytes())
    }

    pub fn utf16(&mut self, s: &str) -> &mut Self {
        for unit in s.encode_utf16() {
            self.u16(unit);
        }
        self
    }

    pub fn fragment_header(&mut self) -> &mut Self {
        self.raw(&[0x0f, 1, 1, 0])
    }

    pub fn eof(&mut self) -> &mut Self {
        self.u8(0x00)
    }

    /// An element start without attributes. Template definitions also carry a dependency ID.
    pub fn open(&mut self, name: &str, in_template_definition: bool) -> &mut Self {
        self.element_start(in_template_definition).name(name)
    }

    /// An element start like `open`'s, with attributes to follow.
    pub fn open_with_attributes(&mut self, name: &str, in_template_definition: bool) -> &mut Self {
        let token = self.bytes.len();
        self.open(name, in_template_definition);
        self.bytes[token] |= 0x40;
        // The attribute list's size, which readers skip.
        self.u32(0)
    }

    /// An element start sharing the name `open` or `attribute` wrote last.
    pub fn reopen(&mut self, in_template_definition: bool) -> &mut Self {
        let name = self.name.expect("a name first");
        self.element_start(in_template_definition).u32(name)
    }

    /// An attribute, whose value follows.
    pub fn attribute(&mut self, name: &str) -> &mut Self {
        self.u8(0x06).name(name)
    }

    /// An attribute sharing the name `open` or `attribute` wrote last.
    pub fn reattribute(&mut self) -> &mut Self {
        let name = self.name.expect("a name first");
        self.u8(0x06).u32(name)
    }

    /// A name reference whose string follows inline: next string, hash, length, UTF-16, NUL.
    fn name(&mut self, name: &str) -> &mut Self {
        let name_offset = self.pos() + 4;
        self.name = Some(name_offset);
        self.u32(name_offset)
            .u32(0)
            .u16(0)
            .u16(name.encode_utf16().count() as u16)
            .utf16(name)
            .u16(0)
    }

    fn element_start(&mut self, in_template_definition: bool) -> &mut Self {
        self.u8(0x01);
        if in_template_definition {
            self.u16(0xffff);
        }
        // The element's data size, which readers skip.
        self.u32(0)
    }

    pub fn close_start(&mut self) -> &mut Self {
        self.u8(0x02)
    }

    pub fn close_empty(&mut self) -> &mut Self {
        self.u8(0x03)
    }

    pub fn end_element(&mut self) -> &mut Self {
        self.u8(0x04)
    }

    /// `<name>text</name>`, outside template definitions.
    pub fn text_element(&mut self, name: &str, text: &str) -> &mut Self {
        self.open(name, false)
            .close_start()
            .text(text)
            .end_element()
    }

    /// A NULL value.
    pub fn null(&mut self) -> &mut Self {
        self.u8(0x05).u8(0x00)
    }

    /// A string value.
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.u8(0x05)
            .u8(0x01)
            .u16(text.encode_utf16().count() as u16)
            .utf16(text)
    }

    /// A UInt32 value.
    pub fn uint32(&mut self, v: u32) -> &mut Self {
        self.u8(0x05).u8(0x08).u32(v)
    }

    /// A BinXML value whose fragment `write` writes. In a template definition, the instances in
    /// it are part of the template.
    pub fn binxml(&mut self, write: impl FnOnce(&mut BinXml)) -> &mut Self {
        let mut fragment = BinXml::at(self.pos() + 4);
        fragment.definition = self.definition;
        write(&mut fragment);
        self.definition = fragment.definition;
        self.u8(0x05)
            .u8(0x21)
            .u16(fragment.bytes.len() as u16)
            .raw(&fragment.bytes)
    }

    /// A normal substitution of value `id`, of type `value_type`.
    pub fn substitution(&mut self, id: u16, value_type: u8) -> &mut Self {
        self.u8(0x0d).u16(id).u8(value_type)
    }

    /// A template instance whose definition follows inline: `definition` writes its element,
    /// between the fragment header and EOF. Then its values.
    pub fn template_instance(
        &mut self,
        definition: impl FnOnce(&mut BinXml),
        values: Vec<Value<'_>>,
    ) -> &mut Self {
        self.u8(0x0c).u8(0x01).u32(1);
        let definition_offset = self.pos() + 4;
        self.u32(definition_offset).u32(0).raw(&[0xab; 16]);
        let mut body = BinXml::at(definition_offset + TEMPLATE_DEFINITION_HEADER_SIZE as u32);
        body.definition = self.definition;
        body.fragment_header();
        definition(&mut body);
        body.eof();
        self.u32(body.bytes.len() as u32).raw(&body.bytes);
        self.definition = Some(definition_offset);
        self.values(values)
    }

    /// An instance of the template `template_instance` defined last.
    pub fn template_ref(&mut self, values: Vec<Value<'_>>) -> &mut Self {
        let definition = self.definition.expect("a template_instance first");
        self.u8(0x0c).u8(0x01).u32(1).u32(definition);
        self.values(values)
    }

    /// A template instance's values: their count and descriptors (size, type, padding), then
    /// their bytes.
    fn values(&mut self, values: Vec<Value<'_>>) -> &mut Self {
        let mut at = self.pos() + 4 + 4 * values.len() as u32;
        let mut written = Vec::with_capacity(values.len());
        for (value_type, write) in values {
            let mut bytes = BinXml::at(at);
            bytes.definition = self.definition;
            write(&mut bytes);
            self.definition = bytes.definition;
            at += bytes.bytes.len() as u32;
            written.push((value_type, bytes.bytes));
        }
        self.u32(written.len() as u32);
        for (value_type, bytes) in &written {
            self.u16(bytes.len() as u16).u8(*value_type).u8(0);
        }
        for (_, bytes) in &written {
            self.raw(bytes);
        }
        self
    }
}

/// A one-chunk EVTX file with one record (ID 1), whose BinXML `record` writes.
pub fn synthetic_evtx(record: impl FnOnce(&mut BinXml)) -> Vec<u8> {
    let mut record = Some(record);
    synthetic_evtx_records(1, |_, binxml| record.take().expect("one record")(binxml))
}

/// A one-chunk EVTX file with `count` records, IDs 1 to `count`, whose BinXML `record(i, ..)`
/// writes for the record at index `i`. `template_ref` can use a template an earlier record
/// defined.
pub fn synthetic_evtx_records(count: usize, mut record: impl FnMut(usize, &mut BinXml)) -> Vec<u8> {
    fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }
    let mut file = vec![0; EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE];
    put(&mut file, 0, b"ElfFile\0");
    put(&mut file, 24, &(count as u64 + 1).to_le_bytes()); // next record ID
    put(&mut file, 32, &128u32.to_le_bytes()); // header size
    put(&mut file, 36, &1u16.to_le_bytes()); // minor version
    put(&mut file, 38, &3u16.to_le_bytes()); // major version
    put(&mut file, 40, &(EVTX_FILE_HEADER_SIZE as u16).to_le_bytes()); // header block size
    put(&mut file, 42, &1u16.to_le_bytes()); // chunk count
    let checksum = evtx::checksum_ieee(&file[..120]);
    put(&mut file, 124, &checksum.to_le_bytes());

    let chunk = &mut file[EVTX_FILE_HEADER_SIZE..];
    let mut record_start = EVTX_CHUNK_HEADER_SIZE;
    let mut last_record = record_start;
    let mut definition = None;
    for i in 0..count {
        let mut binxml = BinXml::at((record_start + EVTX_RECORD_HEADER_SIZE) as u32);
        binxml.definition = definition;
        record(i, &mut binxml);
        definition = binxml.definition;
        let record_size = EVTX_RECORD_HEADER_SIZE + binxml.bytes.len() + 4;
        let end = record_start + record_size;
        assert!(
            end <= EVTX_CHUNK_SIZE,
            "record {} overflows the chunk",
            i + 1
        );
        let record = &mut chunk[record_start..end];
        put(record, 0, EVTX_RECORD_MAGIC);
        put(record, 4, &(record_size as u32).to_le_bytes());
        put(record, 8, &(i as u64 + 1).to_le_bytes());
        put(record, 16, &131_000_000_000_000_000u64.to_le_bytes()); // 2016-02-15T08:53:20Z
        put(record, EVTX_RECORD_HEADER_SIZE, &binxml.bytes);
        put(record, record_size - 4, &(record_size as u32).to_le_bytes());
        last_record = record_start;
        record_start = end;
    }
    let free_space = record_start;

    put(chunk, 0, b"ElfChnk\0");
    // First and last record numbers, then IDs.
    for (at, id) in [(8, 1), (16, count), (24, 1), (32, count)] {
        put(chunk, at, &(id as u64).to_le_bytes());
    }
    put(chunk, 40, &128u32.to_le_bytes()); // header size
    put(chunk, 44, &(last_record as u32).to_le_bytes()); // last record offset
    put(chunk, 48, &(free_space as u32).to_le_bytes());

    let events_checksum = evtx::checksum_ieee(&chunk[EVTX_CHUNK_HEADER_SIZE..free_space]);
    put(chunk, 52, &events_checksum.to_le_bytes());
    let header_checksum =
        evtx::checksum_ieee(&[&chunk[..120], &chunk[128..EVTX_CHUNK_HEADER_SIZE]].concat());
    put(chunk, 124, &header_checksum.to_le_bytes());
    file
}

/// An `.MTA` file with one publisher, `provider`, whose level 0 is named `level`.
pub fn mta_naming_level(provider: &str, level: &str) -> Vec<u8> {
    // `text` in UTF-16, after its size in bytes.
    fn counted(text: &str) -> Vec<u8> {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut counted = (utf16.len() as u32).to_le_bytes().to_vec();
        counted.extend(utf16);
        counted
    }
    // One page whose one entry, number 0, follows its header. Its next page is itself: the last.
    fn section(entry: Vec<u8>) -> Vec<u8> {
        const PAGE_HEADER_SIZE: usize = 888;
        // Where the page header lists the offsets of the page's entries.
        const PAGE_ENTRIES: usize = 88;
        let mut page = vec![0; PAGE_HEADER_SIZE];
        page[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        page[PAGE_ENTRIES..][..8].copy_from_slice(&(PAGE_HEADER_SIZE as u64).to_le_bytes());
        page.extend(0u32.to_le_bytes());
        page.extend((entry.len() as u32).to_le_bytes());
        page.extend(entry);
        page
    }
    // Level 0 named by message 0, then no tasks, opcodes or keywords, then the name.
    let mut publisher: Vec<u8> = [1u32, 0, 0, 0, 0, 0].map(u32::to_le_bytes).concat();
    publisher.extend(counted(provider));
    let sections = [
        (counted("EVT"), Vec::new()),
        (counted("MSG"), section(counted(level))),
        (counted("PUB"), section(publisher)),
    ];
    // Each section's descriptor: its offset and size (u64 each), then its name.
    let table: usize = sections.iter().map(|(name, _)| 16 + name.len()).sum();
    let mut file = b"MTAFile\0".to_vec();
    // Version 1.1, then the size and length of the section table.
    for value in [1, 1, table as u32, sections.len() as u32] {
        file.extend(u32::to_le_bytes(value));
    }
    let mut offset = file.len() + table;
    for (name, data) in &sections {
        file.extend((offset as u64).to_le_bytes());
        file.extend((data.len() as u64).to_le_bytes());
        file.extend(name);
        offset += data.len();
    }
    for (_, data) in sections {
        file.extend(data);
    }
    file
}
