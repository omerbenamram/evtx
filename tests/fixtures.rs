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
}

impl BinXml {
    fn at(base: u32) -> Self {
        BinXml {
            bytes: Vec::new(),
            base,
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

    fn utf16(&mut self, s: &str) -> &mut Self {
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
        self.u8(0x01);
        if in_template_definition {
            self.u16(0xffff);
        }
        // The element's data size, which readers skip.
        self.u32(0);
        // A name reference whose string follows inline: next string, hash, length, UTF-16, NUL.
        let name_offset = self.pos() + 4;
        self.u32(name_offset)
            .u32(0)
            .u16(0)
            .u16(name.encode_utf16().count() as u16)
            .utf16(name)
            .u16(0)
    }

    pub fn close_start(&mut self) -> &mut Self {
        self.u8(0x02)
    }

    pub fn end_element(&mut self) -> &mut Self {
        self.u8(0x04)
    }

    /// `<name>text</name>`, outside template definitions.
    pub fn text_element(&mut self, name: &str, text: &str) -> &mut Self {
        self.open(name, false).close_start();
        self.u8(0x05)
            .u8(0x01)
            .u16(text.encode_utf16().count() as u16)
            .utf16(text);
        self.end_element()
    }

    /// A normal substitution of value `id`, of type `value_type`.
    pub fn substitution(&mut self, id: u16, value_type: u8) -> &mut Self {
        self.u8(0x0d).u16(id).u8(value_type)
    }

    /// A template instance whose definition follows inline: `definition` writes its element,
    /// between the fragment header and EOF. Its one value, of type `value_type`, is written by
    /// `value` at the offset its bytes land at.
    pub fn template_instance(
        &mut self,
        definition: impl FnOnce(&mut BinXml),
        value_type: u8,
        value: impl FnOnce(&mut BinXml),
    ) -> &mut Self {
        self.u8(0x0c).u8(0x01).u32(1);
        let definition_offset = self.pos() + 4;
        self.u32(definition_offset).u32(0).raw(&[0xab; 16]);
        let mut body = BinXml::at(definition_offset + TEMPLATE_DEFINITION_HEADER_SIZE as u32);
        body.fragment_header();
        definition(&mut body);
        body.eof();
        self.u32(body.bytes.len() as u32).raw(&body.bytes);

        // The value count and descriptor (size, type, padding), then the value.
        self.u32(1);
        let mut bytes = BinXml::at(self.pos() + 4);
        value(&mut bytes);
        self.u16(bytes.bytes.len() as u16)
            .u8(value_type)
            .u8(0)
            .raw(&bytes.bytes)
    }
}

/// A one-chunk EVTX file with one record (ID 1), whose BinXML `record` writes.
pub fn synthetic_evtx(record: impl FnOnce(&mut BinXml)) -> Vec<u8> {
    fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }
    let record_start = EVTX_CHUNK_HEADER_SIZE;
    let mut binxml = BinXml::at((record_start + EVTX_RECORD_HEADER_SIZE) as u32);
    record(&mut binxml);

    let mut file = vec![0; EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE];
    put(&mut file, 0, b"ElfFile\0");
    put(&mut file, 24, &2u64.to_le_bytes()); // next record ID
    put(&mut file, 32, &128u32.to_le_bytes()); // header size
    put(&mut file, 36, &1u16.to_le_bytes()); // minor version
    put(&mut file, 38, &3u16.to_le_bytes()); // major version
    put(&mut file, 40, &(EVTX_FILE_HEADER_SIZE as u16).to_le_bytes()); // header block size
    put(&mut file, 42, &1u16.to_le_bytes()); // chunk count
    let checksum = evtx::checksum_ieee(&file[..120]);
    put(&mut file, 124, &checksum.to_le_bytes());

    let chunk = &mut file[EVTX_FILE_HEADER_SIZE..];
    let record_size = EVTX_RECORD_HEADER_SIZE + binxml.bytes.len() + 4;
    let free_space = record_start + record_size;
    put(chunk, 0, b"ElfChnk\0");
    // First and last record numbers and IDs.
    for at in [8, 16, 24, 32] {
        put(chunk, at, &1u64.to_le_bytes());
    }
    put(chunk, 40, &128u32.to_le_bytes()); // header size
    put(chunk, 44, &(record_start as u32).to_le_bytes()); // last record offset
    put(chunk, 48, &(free_space as u32).to_le_bytes());

    let record = &mut chunk[record_start..free_space];
    put(record, 0, EVTX_RECORD_MAGIC);
    put(record, 4, &(record_size as u32).to_le_bytes());
    put(record, 8, &1u64.to_le_bytes());
    put(record, 16, &131_000_000_000_000_000u64.to_le_bytes()); // 2016-02-15T08:53:20Z
    put(record, EVTX_RECORD_HEADER_SIZE, &binxml.bytes);
    put(record, record_size - 4, &(record_size as u32).to_le_bytes());

    let events_checksum = evtx::checksum_ieee(&chunk[record_start..free_space]);
    put(chunk, 52, &events_checksum.to_le_bytes());
    let header_checksum =
        evtx::checksum_ieee(&[&chunk[..120], &chunk[128..EVTX_CHUNK_HEADER_SIZE]].concat());
    put(chunk, 124, &header_checksum.to_le_bytes());
    file
}
