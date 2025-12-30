#![allow(clippy::result_large_err)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bumpalo::Bump;
use serde_json::Value as JsonValue;
use thiserror::Error;

use super::manifest::WevtManifestError;

#[derive(Debug, Error)]
pub enum WevtCacheError {
    #[error("failed to read WEVT cache index `{path}`: {source}")]
    ReadIndex { path: PathBuf, source: io::Error },

    #[error("invalid JSONL at {path}:{line_no}: {source}")]
    InvalidJsonLine {
        path: PathBuf,
        line_no: usize,
        source: serde_json::Error,
    },

    #[error("failed to read cache blob `{path}`: {source}")]
    ReadBlob { path: PathBuf, source: io::Error },

    #[error(
        "TEMP slice out of bounds for `{path}` (offset={temp_offset}, size={temp_size}, len={len})"
    )]
    TempSliceOutOfBounds {
        path: PathBuf,
        temp_offset: u32,
        temp_size: u32,
        len: usize,
    },

    #[error("failed to parse CRIM/WEVT blob `{path}` while scanning templates: {source}")]
    CrimParse {
        path: PathBuf,
        source: WevtManifestError,
    },

    #[error(
        "template GUID `{guid}` not found in cache index `{index_path}` (and not discovered in any CRIM blobs)"
    )]
    TemplateNotFound { guid: String, index_path: PathBuf },

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
    /// A standalone TEMP blob written by `extract-wevt-templates --split-ttbl`.
    TempFile(PathBuf),
    /// A TEMP slice located inside a CRIM/WEVT blob (offset/size refer to the blob bytes).
    CrimSlice {
        path: PathBuf,
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
/// The cache index format is produced by the `evtx_dump extract-wevt-templates` subcommand.
#[derive(Debug)]
pub struct WevtCache {
    index_path: PathBuf,
    crim_paths: Vec<PathBuf>,

    sources_by_guid: Mutex<HashMap<String, TemplateSource>>,
    scanned_crims: Mutex<HashSet<PathBuf>>,
    blob_cache: Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>,
}

impl WevtCache {
    /// Load a cache index JSONL produced by `evtx_dump extract-wevt-templates`.
    pub fn load(index_path: impl AsRef<Path>) -> Result<Self, WevtCacheError> {
        let index_path = index_path.as_ref().to_path_buf();
        let text = fs::read_to_string(&index_path).map_err(|source| WevtCacheError::ReadIndex {
            path: index_path.clone(),
            source,
        })?;

        let mut crim_paths: Vec<PathBuf> = Vec::new();
        let mut sources_by_guid: HashMap<String, TemplateSource> = HashMap::new();

        fn resolve_output_path(index_path: &Path, output_path: &str) -> PathBuf {
            let p = Path::new(output_path);
            if p.is_absolute() {
                return p.to_path_buf();
            }
            let base = index_path.parent().unwrap_or_else(|| Path::new("."));
            base.join(p)
        }

        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let v: JsonValue =
                serde_json::from_str(line).map_err(|source| WevtCacheError::InvalidJsonLine {
                    path: index_path.clone(),
                    line_no: line_no + 1,
                    source,
                })?;

            // ExtractWevtTemplatesOutputLine: has output_path + size, but no guid/provider_guid/template_guid.
            if v.get("output_path").and_then(|p| p.as_str()).is_some()
                && v.get("size").is_some()
                && v.get("guid").is_none()
                && v.get("provider_guid").is_none()
                && v.get("template_guid").is_none()
            {
                if let Some(p) = v.get("output_path").and_then(|p| p.as_str()) {
                    crim_paths.push(resolve_output_path(&index_path, p));
                }
                continue;
            }

            // ExtractWevtTempOutputLine: has guid + temp_offset/temp_size + output_path to a standalone TEMP blob.
            if let (Some(guid), Some(output_path)) = (
                v.get("guid").and_then(|v| v.as_str()),
                v.get("output_path").and_then(|v| v.as_str()),
            ) {
                // Only accept this as a TEMP binary when the expected TEMP fields exist.
                if v.get("temp_offset").is_some() && v.get("temp_size").is_some() {
                    sources_by_guid.insert(
                        normalize_guid(guid),
                        TemplateSource::TempFile(resolve_output_path(&index_path, output_path)),
                    );
                }
            }
        }

        // De-dupe CRIM paths (index can contain repeats).
        crim_paths.sort();
        crim_paths.dedup();

        Ok(WevtCache {
            index_path,
            crim_paths,
            sources_by_guid: Mutex::new(sources_by_guid),
            scanned_crims: Mutex::new(HashSet::new()),
            blob_cache: Mutex::new(HashMap::new()),
        })
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
        // Fast path: do we already know a source for this guid?
        if let Some(tb) = self.try_load_from_known_source(guid)? {
            return Ok(tb);
        }

        // Otherwise, scan CRIM blobs until we discover the guid.
        for crim_path in &self.crim_paths {
            if self.is_scanned(crim_path) {
                continue;
            }
            self.scan_crim_for_templates(crim_path)?;

            if let Some(tb) = self.try_load_from_known_source(guid)? {
                return Ok(tb);
            }
        }

        Err(WevtCacheError::TemplateNotFound {
            guid: guid.to_string(),
            index_path: self.index_path.clone(),
        })
    }

    fn is_scanned(&self, path: &Path) -> bool {
        self.scanned_crims
            .lock()
            .expect("lock poisoned")
            .contains(path)
    }

    fn mark_scanned(&self, path: &Path) {
        self.scanned_crims
            .lock()
            .expect("lock poisoned")
            .insert(path.to_path_buf());
    }

    fn load_blob(&self, path: &Path) -> Result<Arc<Vec<u8>>, WevtCacheError> {
        if let Some(existing) = self
            .blob_cache
            .lock()
            .expect("lock poisoned")
            .get(path)
            .cloned()
        {
            return Ok(existing);
        }

        let bytes = fs::read(path).map_err(|source| WevtCacheError::ReadBlob {
            path: path.to_path_buf(),
            source,
        })?;
        let bytes = Arc::new(bytes);

        self.blob_cache
            .lock()
            .expect("lock poisoned")
            .insert(path.to_path_buf(), bytes.clone());

        Ok(bytes)
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
            TemplateSource::TempFile(path) => {
                let bytes = self.load_blob(&path)?;
                Ok(Some(TempBytes {
                    bytes: bytes.clone(),
                    start: 0,
                    end: bytes.len(),
                }))
            }
            TemplateSource::CrimSlice {
                path,
                temp_offset,
                temp_size,
            } => {
                let bytes = self.load_blob(&path)?;
                let start = temp_offset as usize;
                let end = start.saturating_add(temp_size as usize);
                if end > bytes.len() {
                    return Err(WevtCacheError::TempSliceOutOfBounds {
                        path,
                        temp_offset,
                        temp_size,
                        len: bytes.len(),
                    });
                }
                Ok(Some(TempBytes { bytes, start, end }))
            }
        }
    }

    fn scan_crim_for_templates(&self, crim_path: &Path) -> Result<(), WevtCacheError> {
        let bytes = self.load_blob(crim_path)?;

        let templates =
            match crate::wevt_templates::extract_temp_templates_from_wevt_blob(bytes.as_slice()) {
                Ok(t) => t,
                Err(source) => {
                    // Mark scanned so we don't repeatedly try a broken blob.
                    self.mark_scanned(crim_path);
                    return Err(WevtCacheError::CrimParse {
                        path: crim_path.to_path_buf(),
                        source,
                    });
                }
            };

        let mut map = self.sources_by_guid.lock().expect("lock poisoned");
        for t in templates {
            let g = normalize_guid(&t.header.guid.to_string());
            map.entry(g).or_insert(TemplateSource::CrimSlice {
                path: crim_path.to_path_buf(),
                temp_offset: t.temp_offset,
                temp_size: t.temp_size,
            });
        }

        self.mark_scanned(crim_path);
        Ok(())
    }
}

pub fn normalize_guid(s: &str) -> String {
    s.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}
