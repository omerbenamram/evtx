#![allow(dead_code)]
use std::path::PathBuf;

use std::sync::Mutex;
use std::sync::Once;

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

/// Splits `evtx_dump`'s chunk output into `(event_record_id, rendered record)` pairs.
pub fn rendered_records(chunks: impl Iterator<Item = evtx::RenderedChunk>) -> Vec<(u64, String)> {
    let mut records = Vec::new();
    for chunk in chunks {
        let mut start = 0;
        for item in chunk.items {
            if let evtx::RenderedChunkItem::Record {
                event_record_id,
                end,
            } = item
            {
                // Each record's bytes end with a newline.
                let data = String::from_utf8(chunk.data[start..end - 1].to_vec()).unwrap();
                records.push((event_record_id, data));
                start = end;
            }
        }
    }
    records
}

/// Chunk offset of the BinXML of [`synthetic_evtx`]'s record, after the chunk header and the
/// record header.
pub const SYNTHETIC_BINXML_OFFSET: u32 = 512 + 24;

/// A template instance's substitution value: its type and a writer for its bytes.
pub type SubstitutionValue<'v> = (u8, &'v dyn Fn(&mut BinXml));

/// BinXML for synthetic records, written at a known chunk offset: inline names and inline
/// template definitions point at their own offset.
pub struct BinXml {
    pub bytes: Vec<u8>,
    base: u32,
}

impl BinXml {
    /// The BinXML of [`synthetic_evtx`]'s record.
    pub fn record() -> Self {
        Self::at(SYNTHETIC_BINXML_OFFSET)
    }

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
    /// between the fragment header and EOF. Each value's writer is called at the offset its
    /// bytes land at.
    pub fn template_instance(
        &mut self,
        definition: impl FnOnce(&mut BinXml),
        values: &[SubstitutionValue<'_>],
    ) -> &mut Self {
        self.u8(0x0c).u8(0x01).u32(1);
        let definition_offset = self.pos() + 4;
        // The definition's header: next template, GUID, data size.
        self.u32(definition_offset).u32(0).raw(&[0xab; 16]);
        let mut body = BinXml::at(definition_offset + 24);
        body.fragment_header();
        definition(&mut body);
        body.eof();
        self.u32(body.bytes.len() as u32).raw(&body.bytes);

        self.u32(values.len() as u32);
        let mut value_offset = self.pos() + 4 * values.len() as u32;
        let mut written = Vec::new();
        for (value_type, write) in values {
            let mut value = BinXml::at(value_offset);
            write(&mut value);
            value_offset += value.bytes.len() as u32;
            self.u16(value.bytes.len() as u16).u8(*value_type).u8(0);
            written.push(value.bytes);
        }
        for value in written {
            self.raw(&value);
        }
        self
    }
}

/// A one-chunk EVTX file with one record (ID 1) holding `binxml`, which must have been
/// written at [`SYNTHETIC_BINXML_OFFSET`] (see [`BinXml::record`]).
pub fn synthetic_evtx(binxml: &[u8]) -> Vec<u8> {
    fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }
    let mut file = vec![0; 4096 + 65536];
    put(&mut file, 0, b"ElfFile\0");
    put(&mut file, 24, &2u64.to_le_bytes()); // next record ID
    put(&mut file, 32, &128u32.to_le_bytes()); // header size
    put(&mut file, 36, &1u16.to_le_bytes()); // minor version
    put(&mut file, 38, &3u16.to_le_bytes()); // major version
    put(&mut file, 40, &4096u16.to_le_bytes()); // header block size
    put(&mut file, 42, &1u16.to_le_bytes()); // chunk count
    let checksum = evtx::checksum_ieee(&file[..120]);
    put(&mut file, 124, &checksum.to_le_bytes());

    let chunk = &mut file[4096..];
    let record_size = (24 + binxml.len() + 4) as u32;
    let free_space = 512 + record_size;
    put(chunk, 0, b"ElfChnk\0");
    // First and last record numbers and IDs.
    for at in [8, 16, 24, 32] {
        put(chunk, at, &1u64.to_le_bytes());
    }
    put(chunk, 40, &128u32.to_le_bytes()); // header size
    put(chunk, 44, &512u32.to_le_bytes()); // last record offset
    put(chunk, 48, &free_space.to_le_bytes());

    let record = &mut chunk[512..free_space as usize];
    put(record, 0, b"**\0\0");
    put(record, 4, &record_size.to_le_bytes());
    put(record, 8, &1u64.to_le_bytes());
    put(record, 16, &131_000_000_000_000_000u64.to_le_bytes()); // 2016-02-15T08:53:20Z
    put(record, 24, binxml);
    put(record, record_size as usize - 4, &record_size.to_le_bytes());

    let events_checksum = evtx::checksum_ieee(&chunk[512..free_space as usize]);
    put(chunk, 52, &events_checksum.to_le_bytes());
    let header_checksum = evtx::checksum_ieee(&[&chunk[..120], &chunk[128..512]].concat());
    put(chunk, 124, &header_checksum.to_le_bytes());
    file
}
