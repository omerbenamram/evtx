#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::path::Path;

use bumpalo::Bump;
use thiserror::Error;

use super::TEMP_HEADER_SIZE;
use super::manifest::{CrimManifest, WevtManifestError};
use super::wevtcache::{ENTRY_HEADER_SIZE, HEADER_SIZE, WevtCacheFileError, for_each_crim_blob};

#[derive(Debug, Error)]
pub enum WevtCacheError {
    #[error(
        "TEMP slice out of bounds (crim_index={crim_index}, offset={temp_offset}, size={temp_size}, len={len})"
    )]
    TempSliceOutOfBounds {
        crim_index: usize,
        temp_offset: u32,
        temp_size: u32,
        len: usize,
    },

    #[error("failed to parse CRIM/WEVT blob: {source}")]
    CrimParse { source: WevtManifestError },

    #[error("template GUID `{guid}` not found in cache")]
    TemplateNotFound { guid: String },

    #[error(
        "TEMP too small to contain BinXML fragment for template_guid={guid} (len={len}, need >= {need})"
    )]
    TempTooSmall {
        guid: String,
        len: usize,
        need: usize,
    },
}

#[derive(Debug)]
enum TemplateSource {
    /// A standalone TEMP blob stored in memory.
    TempBytes(Vec<u8>),
    /// A TEMP slice located inside a CRIM/WEVT blob (offset/size refer to the blob bytes).
    CrimSlice {
        crim_index: usize,
        temp_offset: u32,
        temp_size: u32,
    },
}

/// Offline cache for extracted `WEVT_TEMPLATE` templates, keyed by template GUID.
///
/// This is primarily intended for "offline rendering" workflows:
/// - Extract WEVT templates from provider binaries into a cache directory + JSONL index.
/// - Use this cache to render EVTX records when their embedded template definitions are missing or
///   fail to deserialize.
///
/// The cache is populated up front (via `&mut self`) and then shared read-only (e.g. as
/// `Arc<WevtCache>`).
#[derive(Debug, Default)]
pub struct WevtCache {
    /// Stored CRIM/WEVT blobs in memory.
    ///
    /// Templates discovered from these blobs can be referenced via [`TemplateSource::CrimSlice`]
    /// without copying.
    crim_blobs: Vec<Vec<u8>>,

    /// Template GUID -> template source.
    sources_by_guid: HashMap<String, TemplateSource>,
}

impl WevtCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        WevtCache::default()
    }

    /// Load a `.wevtcache` file and index all of its `CRIM` blobs into a [`WevtCache`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self, WevtCacheFileError> {
        let path = path.as_ref();
        let mut cache = Self::new();
        let mut index = 0u32;
        let mut next_entry_offset = HEADER_SIZE as u64;
        for_each_crim_blob(path, |bytes| {
            let offset = next_entry_offset + ENTRY_HEADER_SIZE as u64;
            next_entry_offset = offset + bytes.len() as u64;
            let cur_index = index;
            index += 1;
            cache
                .add_wevt_blob(bytes)
                .map_err(|source| WevtCacheFileError::InvalidEntry {
                    path: path.to_path_buf(),
                    index: cur_index,
                    offset,
                    source,
                })?;
            Ok(())
        })?;
        Ok(cache)
    }

    /// Insert a standalone TEMP blob (full TEMP bytes) into this cache.
    ///
    /// `template_guid` is normalized (case-insensitive, braces stripped).
    pub fn insert_temp_bytes(&mut self, template_guid: &str, temp_bytes: Vec<u8>) {
        let guid = normalize_guid(template_guid);
        self.sources_by_guid
            .insert(guid, TemplateSource::TempBytes(temp_bytes));
    }

    /// Add a CRIM/WEVT blob to this cache and index all contained `TTBL/TEMP` entries.
    ///
    /// This is **strict**: parse failures return an error and do not modify the cache.
    pub fn add_wevt_blob(&mut self, bytes: Vec<u8>) -> Result<usize, WevtCacheError> {
        let manifest =
            CrimManifest::parse(&bytes).map_err(|source| WevtCacheError::CrimParse { source })?;

        let crim_index = self.crim_blobs.len();
        let mut inserted = 0usize;
        for tpl in manifest.templates() {
            // First source wins for stability.
            let entry = self
                .sources_by_guid
                .entry(normalize_guid(&tpl.guid.to_string()));
            if let std::collections::hash_map::Entry::Vacant(v) = entry {
                v.insert(TemplateSource::CrimSlice {
                    crim_index,
                    temp_offset: tpl.offset,
                    temp_size: tpl.size,
                });
                inserted += 1;
            }
        }

        self.crim_blobs.push(bytes);
        Ok(inserted)
    }

    /// Load the raw TEMP BinXML fragment (starting at offset 40) into `arena` and return it.
    ///
    /// The returned slice has the lifetime of `arena`, making it suitable for parsing into
    /// IR structures that borrow from the template bytes.
    pub(crate) fn load_temp_binxml_fragment_in<'a>(
        &self,
        template_guid: &str,
        arena: &'a Bump,
    ) -> Result<&'a [u8], WevtCacheError> {
        let guid = normalize_guid(template_guid);
        let temp = self.get_temp_bytes_for_guid(&guid)?;

        if temp.len() < TEMP_HEADER_SIZE {
            return Err(WevtCacheError::TempTooSmall {
                guid,
                len: temp.len(),
                need: TEMP_HEADER_SIZE,
            });
        }

        Ok(arena.alloc_slice_copy(&temp[TEMP_HEADER_SIZE..]))
    }

    fn get_temp_bytes_for_guid(&self, guid: &str) -> Result<&[u8], WevtCacheError> {
        let Some(src) = self.sources_by_guid.get(guid) else {
            return Err(WevtCacheError::TemplateNotFound {
                guid: guid.to_string(),
            });
        };

        match *src {
            TemplateSource::TempBytes(ref bytes) => Ok(bytes),
            TemplateSource::CrimSlice {
                crim_index,
                temp_offset,
                temp_size,
            } => {
                let bytes = self
                    .crim_blobs
                    .get(crim_index)
                    .expect("crim_index out of bounds");
                let start = temp_offset as usize;
                let end = start.saturating_add(temp_size as usize);
                if end > bytes.len() {
                    return Err(WevtCacheError::TempSliceOutOfBounds {
                        crim_index,
                        temp_offset,
                        temp_size,
                        len: bytes.len(),
                    });
                }
                Ok(&bytes[start..end])
            }
        }
    }
}

pub fn normalize_guid(s: &str) -> String {
    s.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wevt_templates::wevtcache::WevtCacheWriter;

    fn minimal_crim_blob() -> Vec<u8> {
        let mut crim = Vec::with_capacity(16);
        crim.extend_from_slice(b"CRIM");
        crim.extend_from_slice(&16u32.to_le_bytes()); // size
        crim.extend_from_slice(&1u16.to_le_bytes()); // major_version
        crim.extend_from_slice(&0u16.to_le_bytes()); // minor_version
        crim.extend_from_slice(&0u32.to_le_bytes()); // provider_count
        crim
    }

    #[test]
    fn load_valid_and_invalid_wevtcache() {
        let dir = tempfile::tempdir().unwrap();
        let valid_path = dir.path().join("valid.wevtcache");
        let mut writer = WevtCacheWriter::create(&valid_path, false).unwrap();
        writer.write_crim_blob(&minimal_crim_blob()).unwrap();
        writer.finish().unwrap();

        let cache = WevtCache::load(&valid_path).unwrap();
        assert_eq!(cache.crim_blobs.len(), 1);
        assert!(cache.sources_by_guid.is_empty());

        let invalid_path = dir.path().join("invalid.wevtcache");
        let mut writer = WevtCacheWriter::create(&invalid_path, false).unwrap();
        let first = minimal_crim_blob();
        let expected_second_offset =
            (HEADER_SIZE + ENTRY_HEADER_SIZE + first.len() + ENTRY_HEADER_SIZE) as u64;
        writer.write_crim_blob(&first).unwrap();
        writer.write_crim_blob(b"not-a-crim-blob").unwrap();
        writer.finish().unwrap();

        let err = WevtCache::load(&invalid_path).unwrap_err();
        assert!(
            matches!(
                err,
                WevtCacheFileError::InvalidEntry {
                    index: 1,
                    offset,
                    source: WevtCacheError::CrimParse { .. },
                    ..
                } if offset == expected_second_offset
            ),
            "{err}"
        );
    }
}
