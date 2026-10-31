//! `.wevtcache` single-file container format for offline `WEVT_TEMPLATE` blobs.
//!
//! This module intentionally lives in the library crate so the same implementation can be reused
//! by:
//! - `evtx_dump extract-wevt-templates` (writer)
//! - `evtx_dump --wevt-cache` / `apply-wevt-cache` (reader)
//! - language bindings (e.g. `pyevtx-rs`) without reimplementing the binary format
//!
//! ## Format (version 1)
//!
//! A `.wevtcache` file is a small, self-describing TLV container:
//!
//! - `MAGIC` (8 bytes): `b"WEVTCACH"`
//! - `version` (u32 LE): currently `1`
//! - `entry_count` (u32 LE)
//! - `entry_count` times:
//!   - `kind` (u8)
//!   - `len` (u64 LE)
//!   - `payload` (`len` bytes)
//!
//! Currently the only supported entry kind is [`EntryKind::Crim`], whose payload is the raw
//! `WEVT_TEMPLATE` resource bytes (a CRIM/WEVT manifest blob).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::utils::bytes;

pub(crate) const MAGIC: [u8; 8] = *b"WEVTCACH";
pub(crate) const VERSION: u32 = 1;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Raw `WEVT_TEMPLATE` resource bytes (CRIM/WEVT payload).
    Crim = 1,
}

#[derive(Debug, Error)]
pub enum WevtCacheFileError {
    #[error("refusing to overwrite existing output file `{path}` (pass overwrite=true)")]
    OutputExists { path: PathBuf },

    #[error("I/O error while {action} `{path}`: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid `.wevtcache` magic for `{path}`")]
    InvalidMagic { path: PathBuf, found: [u8; 8] },

    #[error("unsupported `.wevtcache` version {found} for `{path}` (expected {expected})")]
    UnsupportedVersion {
        path: PathBuf,
        found: u32,
        expected: u32,
    },

    #[error("unknown `.wevtcache` entry kind {kind} for `{path}`")]
    UnknownEntryKind { path: PathBuf, kind: u8 },

    /// The entry's declared length is larger than the bytes left in the file.
    #[error("`.wevtcache` entry length {len} is larger than the rest of `{path}`")]
    EntryLengthTooLarge { path: PathBuf, len: u64 },

    #[error("wevtcache entry count overflow for `{path}`")]
    EntryCountOverflow { path: PathBuf },
}

pub type Result<T> = std::result::Result<T, WevtCacheFileError>;

/// Builds the error lazily, so the path is only copied when I/O fails.
fn io_err(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> WevtCacheFileError {
    move |source| WevtCacheFileError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

/// Header layout: magic (8) + version (u32 LE) + entry_count (u32 LE).
const HEADER_SIZE: usize = 16;
const ENTRY_COUNT_OFFSET: usize = 12;
/// Entry header layout: kind (u8) + len (u64 LE).
const ENTRY_HEADER_SIZE: usize = 9;

pub struct WevtCacheWriter {
    file: File,
    path: PathBuf,
    count: u32,
}

impl WevtCacheWriter {
    pub fn create(path: &Path, overwrite: bool) -> Result<Self> {
        if path.exists() && !overwrite {
            return Err(WevtCacheFileError::OutputExists {
                path: path.to_path_buf(),
            });
        }

        let mut file = File::create(path).map_err(io_err("create", path))?;

        // entry_count is a placeholder, patched by `finish`.
        let mut header = [0u8; HEADER_SIZE];
        header[..8].copy_from_slice(&MAGIC);
        header[8..12].copy_from_slice(&VERSION.to_le_bytes());
        file.write_all(&header)
            .map_err(io_err("write header", path))?;

        Ok(Self {
            file,
            path: path.to_path_buf(),
            count: 0,
        })
    }

    pub fn write_crim_blob(&mut self, bytes: &[u8]) -> Result<()> {
        let mut header = [0u8; ENTRY_HEADER_SIZE];
        header[0] = EntryKind::Crim as u8;
        header[1..].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.file
            .write_all(&header)
            .map_err(io_err("write entry header", &self.path))?;
        self.file
            .write_all(bytes)
            .map_err(io_err("write entry payload", &self.path))?;

        self.count =
            self.count
                .checked_add(1)
                .ok_or_else(|| WevtCacheFileError::EntryCountOverflow {
                    path: self.path.clone(),
                })?;

        Ok(())
    }

    pub fn finish(mut self) -> Result<u32> {
        // Patch entry_count into header.
        self.file
            .seek(SeekFrom::Start(ENTRY_COUNT_OFFSET as u64))
            .map_err(io_err("seek to entry_count", &self.path))?;
        self.file
            .write_all(&self.count.to_le_bytes())
            .map_err(io_err("write entry_count", &self.path))?;
        self.file.flush().map_err(io_err("flush", &self.path))?;

        Ok(self.count)
    }
}

pub struct WevtCacheReader {
    file: File,
    path: PathBuf,
    remaining: u32,
    /// Bytes left in the file after the current position; bounds each entry's
    /// declared length so a crafted header can't force a huge allocation.
    bytes_left: u64,
}

impl WevtCacheReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path).map_err(io_err("open", path))?;

        let mut header = [0u8; HEADER_SIZE];
        file.read_exact(&mut header)
            .map_err(io_err("read header", path))?;
        let u32_at = |o: usize| u32::from_le_bytes(bytes::le(&header[o..]));

        let magic: [u8; 8] = bytes::le(&header);
        if magic != MAGIC {
            return Err(WevtCacheFileError::InvalidMagic {
                path: path.to_path_buf(),
                found: magic,
            });
        }

        let version = u32_at(8);
        if version != VERSION {
            return Err(WevtCacheFileError::UnsupportedVersion {
                path: path.to_path_buf(),
                found: version,
                expected: VERSION,
            });
        }

        let file_len = file.metadata().map_err(io_err("stat", path))?.len();

        Ok(Self {
            file,
            path: path.to_path_buf(),
            remaining: u32_at(ENTRY_COUNT_OFFSET),
            bytes_left: file_len.saturating_sub(HEADER_SIZE as u64),
        })
    }

    pub fn next_entry(&mut self) -> Result<Option<(EntryKind, Vec<u8>)>> {
        if self.remaining == 0 {
            return Ok(None);
        }

        let mut header = [0u8; ENTRY_HEADER_SIZE];
        self.file
            .read_exact(&mut header)
            .map_err(io_err("read entry header", &self.path))?;

        let kind = match header[0] {
            x if x == EntryKind::Crim as u8 => EntryKind::Crim,
            other => {
                return Err(WevtCacheFileError::UnknownEntryKind {
                    path: self.path.clone(),
                    kind: other,
                });
            }
        };

        let len_u64 = u64::from_le_bytes(bytes::le(&header[1..]));
        self.bytes_left = self.bytes_left.saturating_sub(ENTRY_HEADER_SIZE as u64);
        let too_large = || WevtCacheFileError::EntryLengthTooLarge {
            path: self.path.clone(),
            len: len_u64,
        };
        if len_u64 > self.bytes_left {
            return Err(too_large());
        }
        let len = usize::try_from(len_u64).map_err(|_| too_large())?;
        self.bytes_left -= len_u64;

        let mut buf = vec![0u8; len];
        self.file
            .read_exact(&mut buf)
            .map_err(io_err("read entry payload", &self.path))?;

        self.remaining -= 1;
        Ok(Some((kind, buf)))
    }
}

pub fn for_each_crim_blob<F>(path: &Path, mut f: F) -> Result<u32>
where
    F: FnMut(Vec<u8>) -> Result<()>,
{
    let mut reader = WevtCacheReader::open(path)?;
    let mut count = 0u32;

    while let Some((kind, bytes)) = reader.next_entry()? {
        match kind {
            EntryKind::Crim => f(bytes)?,
        }
        // Cannot overflow: bounded by the header's u32 entry_count.
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 25-byte file whose single entry claims a ~4 GiB payload must be
    /// rejected before the payload buffer is allocated.
    #[test]
    fn entry_length_beyond_file_is_rejected_before_allocating() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(EntryKind::Crim as u8);
        bytes.extend_from_slice(&0xFFFF_FFF0u64.to_le_bytes());

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();

        let err = for_each_crim_blob(file.path(), |_| Ok(())).unwrap_err();
        assert!(
            matches!(
                err,
                WevtCacheFileError::EntryLengthTooLarge {
                    len: 0xFFFF_FFF0,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn round_trip_still_reads_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wevtcache");
        let mut w = WevtCacheWriter::create(&path, false).unwrap();
        w.write_crim_blob(b"abc").unwrap();
        w.write_crim_blob(b"").unwrap();
        w.finish().unwrap();

        let mut blobs = Vec::new();
        assert_eq!(
            for_each_crim_blob(&path, |b| {
                blobs.push(b);
                Ok(())
            })
            .unwrap(),
            2
        );
        assert_eq!(blobs, vec![b"abc".to_vec(), Vec::new()]);
    }
}
