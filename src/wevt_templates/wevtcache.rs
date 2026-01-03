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
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

pub const MAGIC: [u8; 8] = *b"WEVTCACH";
pub const VERSION: u32 = 1;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Raw `WEVT_TEMPLATE` resource bytes (CRIM/WEVT payload).
    Crim = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WevtCacheHeader {
    pub magic: [u8; 8],
    pub version: u32,
    pub entry_count: u32,
}

impl WevtCacheHeader {
    pub const SIZE: usize = 8 + 4 + 4;

    pub fn new(entry_count: u32) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            entry_count,
        }
    }

    fn read_from(mut r: impl Read) -> std::io::Result<Self> {
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;

        let mut version_bytes = [0u8; 4];
        r.read_exact(&mut version_bytes)?;
        let version = u32::from_le_bytes(version_bytes);

        let mut count_bytes = [0u8; 4];
        r.read_exact(&mut count_bytes)?;
        let entry_count = u32::from_le_bytes(count_bytes);

        Ok(Self {
            magic,
            version,
            entry_count,
        })
    }

    fn write_to(&self, mut w: impl Write) -> std::io::Result<()> {
        w.write_all(&self.magic)?;
        w.write_all(&self.version.to_le_bytes())?;
        w.write_all(&self.entry_count.to_le_bytes())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WevtCacheEntryHeader {
    pub kind: EntryKind,
    pub len: u64,
}

impl WevtCacheEntryHeader {
    pub const SIZE: usize = 1 + 8;

    fn read_from(mut r: impl Read) -> std::io::Result<(u8, u64)> {
        let mut kind = [0u8; 1];
        r.read_exact(&mut kind)?;

        let mut len_bytes = [0u8; 8];
        r.read_exact(&mut len_bytes)?;
        let len = u64::from_le_bytes(len_bytes);

        Ok((kind[0], len))
    }

    fn write_to(&self, mut w: impl Write) -> std::io::Result<()> {
        w.write_all(&[self.kind as u8])?;
        w.write_all(&self.len.to_le_bytes())?;
        Ok(())
    }
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

    #[error("`.wevtcache` entry length does not fit usize for `{path}`: {len}")]
    EntryLengthTooLarge { path: PathBuf, len: u64 },

    #[error("wevtcache entry count overflow for `{path}`")]
    EntryCountOverflow { path: PathBuf },
}

pub type Result<T> = std::result::Result<T, WevtCacheFileError>;

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

        let mut file = File::create(path).map_err(|e| WevtCacheFileError::Io {
            action: "create",
            path: path.to_path_buf(),
            source: e,
        })?;

        // Header: magic + version + entry_count placeholder.
        WevtCacheHeader::new(0)
            .write_to(&mut file)
            .map_err(|e| WevtCacheFileError::Io {
                action: "write header",
                path: path.to_path_buf(),
                source: e,
            })?;

        Ok(Self {
            file,
            path: path.to_path_buf(),
            count: 0,
        })
    }

    pub fn write_crim_blob(&mut self, bytes: &[u8]) -> Result<()> {
        self.write_entry(EntryKind::Crim, bytes)
    }

    fn write_entry(&mut self, kind: EntryKind, bytes: &[u8]) -> Result<()> {
        WevtCacheEntryHeader {
            kind,
            len: bytes.len() as u64,
        }
        .write_to(&mut self.file)
        .map_err(|e| WevtCacheFileError::Io {
            action: "write entry header",
            path: self.path.clone(),
            source: e,
        })?;

        self.file.write_all(bytes).map_err(|e| WevtCacheFileError::Io {
            action: "write entry payload",
            path: self.path.clone(),
            source: e,
        })?;

        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| WevtCacheFileError::EntryCountOverflow {
                path: self.path.clone(),
            })?;

        Ok(())
    }

    pub fn finish(mut self) -> Result<u32> {
        // Patch entry_count into header.
        let entry_count_offset = MAGIC.len() as u64 + 4;

        self.file
            .seek(SeekFrom::Start(entry_count_offset))
            .map_err(|e| WevtCacheFileError::Io {
                action: "seek to entry_count",
                path: self.path.clone(),
                source: e,
            })?;

        self.file
            .write_all(&self.count.to_le_bytes())
            .map_err(|e| WevtCacheFileError::Io {
                action: "write entry_count",
                path: self.path.clone(),
                source: e,
            })?;

        self.file.flush().map_err(|e| WevtCacheFileError::Io {
            action: "flush",
            path: self.path.clone(),
            source: e,
        })?;

        Ok(self.count)
    }
}

pub struct WevtCacheReader {
    file: File,
    path: PathBuf,
    remaining: u32,
}

impl WevtCacheReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path).map_err(|e| WevtCacheFileError::Io {
            action: "open",
            path: path.to_path_buf(),
            source: e,
        })?;

        let header = WevtCacheHeader::read_from(&mut file).map_err(|e| WevtCacheFileError::Io {
            action: "read header",
            path: path.to_path_buf(),
            source: e,
        })?;

        if header.magic != MAGIC {
            return Err(WevtCacheFileError::InvalidMagic {
                path: path.to_path_buf(),
                found: header.magic,
            });
        }

        if header.version != VERSION {
            return Err(WevtCacheFileError::UnsupportedVersion {
                path: path.to_path_buf(),
                found: header.version,
                expected: VERSION,
            });
        }

        Ok(Self {
            file,
            path: path.to_path_buf(),
            remaining: header.entry_count,
        })
    }

    pub fn next_entry(&mut self) -> Result<Option<(EntryKind, Vec<u8>)>> {
        if self.remaining == 0 {
            return Ok(None);
        }

        let (kind_u8, len_u64) = WevtCacheEntryHeader::read_from(&mut self.file).map_err(|e| {
            WevtCacheFileError::Io {
                action: "read entry header",
                path: self.path.clone(),
                source: e,
            }
        })?;

        let kind = match kind_u8 {
            x if x == EntryKind::Crim as u8 => EntryKind::Crim,
            other => {
                return Err(WevtCacheFileError::UnknownEntryKind {
                    path: self.path.clone(),
                    kind: other,
                })
            }
        };

        let len: usize = usize::try_from(len_u64).map_err(|_| WevtCacheFileError::EntryLengthTooLarge {
            path: self.path.clone(),
            len: len_u64,
        })?;

        let mut buf = vec![0u8; len];
        self.file
            .read_exact(&mut buf)
            .map_err(|e| WevtCacheFileError::Io {
                action: "read entry payload",
                path: self.path.clone(),
                source: e,
            })?;

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
            EntryKind::Crim => {
                f(bytes)?;
                count = count.checked_add(1).ok_or_else(|| WevtCacheFileError::EntryCountOverflow {
                    path: path.to_path_buf(),
                })?;
            }
        }
    }

    Ok(count)
}

/// Variant of [`for_each_crim_blob`] which allows callers to pick their own error type (e.g. `anyhow::Error`).
pub fn for_each_crim_blob_with<F, E>(path: &Path, mut f: F) -> std::result::Result<u32, E>
where
    F: FnMut(Vec<u8>) -> std::result::Result<(), E>,
    E: From<WevtCacheFileError>,
{
    let mut count = 0u32;
    let mut reader = WevtCacheReader::open(path).map_err(E::from)?;

    while let Some((kind, bytes)) = reader.next_entry().map_err(E::from)? {
        match kind {
            EntryKind::Crim => {
                f(bytes)?;
                count = count.checked_add(1).ok_or_else(|| {
                    E::from(WevtCacheFileError::EntryCountOverflow {
                        path: path.to_path_buf(),
                    })
                })?;
            }
        }
    }

    Ok(count)
}

