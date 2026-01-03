#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bumpalo::Bump;
use thiserror::Error;

use super::manifest::WevtManifestError;

#[derive(Debug, Error)]
pub enum WevtCacheError {
    #[error("TEMP slice out of bounds (crim_index={crim_index}, offset={temp_offset}, size={temp_size}, len={len})")]
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

#[derive(Debug, Clone)]
struct TempBytes {
    bytes: Arc<Vec<u8>>,
    start: usize,
    end: usize,
}

impl TempBytes {
    fn as_slice(&self) -> &[u8] {
        &self.bytes[self.start..self.end]
    }
}

#[derive(Debug, Clone)]
enum TemplateSource {
    /// A standalone TEMP blob stored in memory.
    TempBytes(Arc<Vec<u8>>),
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
#[derive(Debug, Default)]
pub struct WevtCache {
    /// Stored CRIM/WEVT blobs in memory.
    ///
    /// Templates discovered from these blobs can be referenced via [`TemplateSource::CrimSlice`]
    /// without copying.
    crim_blobs: Mutex<Vec<Arc<Vec<u8>>>>,

    /// Template GUID -> template source.
    sources_by_guid: Mutex<HashMap<String, TemplateSource>>,
}

impl WevtCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        WevtCache::default()
    }

    /// Backwards-compatible alias for [`WevtCache::new`].
    pub fn in_memory() -> Self {
        WevtCache::new()
    }

    /// Insert a standalone TEMP blob (full TEMP bytes) into this cache.
    ///
    /// `template_guid` is normalized (case-insensitive, braces stripped).
    pub fn insert_temp_bytes(&self, template_guid: &str, temp_bytes: Arc<Vec<u8>>) {
        let guid = normalize_guid(template_guid);
        self.sources_by_guid
            .lock()
            .expect("lock poisoned")
            .insert(guid, TemplateSource::TempBytes(temp_bytes));
    }

    /// Add a CRIM/WEVT blob to this cache and index all contained `TTBL/TEMP` entries.
    ///
    /// This is **strict**: parse failures return an error and do not modify the cache.
    pub fn add_wevt_blob(&self, bytes: Arc<Vec<u8>>) -> Result<usize, WevtCacheError> {
        let templates = crate::wevt_templates::extract_temp_templates_from_wevt_blob(bytes.as_slice())
            .map_err(|source| WevtCacheError::CrimParse { source })?;

        let crim_index = {
            let mut crims = self.crim_blobs.lock().expect("lock poisoned");
            crims.push(bytes);
            crims.len() - 1
        };

        let mut inserted = 0usize;
        let mut map = self.sources_by_guid.lock().expect("lock poisoned");
        for t in templates {
            let g = normalize_guid(&t.header.guid.to_string());
            // First source wins for stability.
            let entry = map.entry(g);
            if let std::collections::hash_map::Entry::Vacant(v) = entry {
                v.insert(TemplateSource::CrimSlice {
                    crim_index,
                    temp_offset: t.temp_offset,
                    temp_size: t.temp_size,
                });
                inserted = inserted.saturating_add(1);
            }
        }

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
        // TEMP layout: first 40 bytes are header, BinXML starts at offset 40.
        const TEMP_BINXML_OFFSET: usize = 40;

        let guid = normalize_guid(template_guid);
        let temp_bytes = self.get_temp_bytes_for_guid(&guid)?;
        let temp = temp_bytes.as_slice();

        if temp.len() < TEMP_BINXML_OFFSET {
            return Err(WevtCacheError::TempTooSmall {
                guid,
                len: temp.len(),
                need: TEMP_BINXML_OFFSET,
            });
        }

        Ok(arena.alloc_slice_copy(&temp[TEMP_BINXML_OFFSET..]))
    }

    fn get_temp_bytes_for_guid(&self, guid: &str) -> Result<TempBytes, WevtCacheError> {
        self.try_load_from_known_source(guid)?
            .ok_or_else(|| WevtCacheError::TemplateNotFound {
                guid: guid.to_string(),
            })
    }

    fn try_load_from_known_source(&self, guid: &str) -> Result<Option<TempBytes>, WevtCacheError> {
        let src = {
            self.sources_by_guid
                .lock()
                .expect("lock poisoned")
                .get(guid)
                .cloned()
        };

        let Some(src) = src else {
            return Ok(None);
        };

        match src {
            TemplateSource::TempBytes(bytes) => {
                Ok(Some(TempBytes {
                    bytes: bytes.clone(),
                    start: 0,
                    end: bytes.len(),
                }))
            }
            TemplateSource::CrimSlice {
                crim_index,
                temp_offset,
                temp_size,
            } => {
                let bytes = self
                    .crim_blobs
                    .lock()
                    .expect("lock poisoned")
                    .get(crim_index)
                    .cloned()
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
                Ok(Some(TempBytes { bytes, start, end }))
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
