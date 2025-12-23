//! CLI-side helper for using an extracted WEVT template cache at render time.
//!
//! This stays in the binary crate (not the library) on purpose: it’s an operational workflow
//! helper and we don’t want to commit to a stable “cache DB” API in `evtx` just yet.

use anyhow::{Context, Result, bail, format_err};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

#[derive(Debug)]
pub struct WevtCache {
    index_path: PathBuf,
    crim_paths: Vec<PathBuf>,

    sources_by_guid: Mutex<HashMap<String, TemplateSource>>,
    scanned_crims: Mutex<HashSet<PathBuf>>,
    blob_cache: Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>,
}

impl WevtCache {
    pub fn load(index_path: impl AsRef<Path>) -> Result<Self> {
        let index_path = index_path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&index_path).with_context(|| {
            format!("failed to read WEVT cache index `{}`", index_path.display())
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

            let v: JsonValue = serde_json::from_str(line).with_context(|| {
                format!("invalid JSONL at {}:{}", index_path.display(), line_no + 1)
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

    pub fn render_by_template_guid(
        &self,
        template_guid: &str,
        substitutions: &[String],
    ) -> Result<String> {
        let guid = normalize_guid(template_guid);
        let temp_bytes = self.get_temp_bytes_for_guid(&guid)?;

        evtx::wevt_templates::render_temp_to_xml_with_substitution_values(
            temp_bytes.as_slice(),
            substitutions,
            encoding::all::WINDOWS_1252,
        )
        .with_context(|| format!("failed to render TEMP for template_guid={guid}"))
    }

    fn get_temp_bytes_for_guid(&self, guid: &str) -> Result<TempBytes> {
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

        Err(format_err!(
            "template GUID `{}` not found in cache index `{}` (and not discovered in any CRIM blobs)",
            guid,
            self.index_path.display()
        ))
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

    fn load_blob(&self, path: &Path) -> Result<Arc<Vec<u8>>> {
        if let Some(existing) = self
            .blob_cache
            .lock()
            .expect("lock poisoned")
            .get(path)
            .cloned()
        {
            return Ok(existing);
        }

        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read cache blob `{}`", path.display()))?;
        let bytes = Arc::new(bytes);

        self.blob_cache
            .lock()
            .expect("lock poisoned")
            .insert(path.to_path_buf(), bytes.clone());

        Ok(bytes)
    }

    fn try_load_from_known_source(&self, guid: &str) -> Result<Option<TempBytes>> {
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
                    bail!(
                        "TEMP slice out of bounds for `{}` (offset={}, size={}, len={})",
                        path.display(),
                        temp_offset,
                        temp_size,
                        bytes.len()
                    );
                }
                Ok(Some(TempBytes { bytes, start, end }))
            }
        }
    }

    fn scan_crim_for_templates(&self, crim_path: &Path) -> Result<()> {
        let bytes = self.load_blob(crim_path)?;

        let templates =
            match evtx::wevt_templates::extract_temp_templates_from_wevt_blob(bytes.as_slice()) {
                Ok(t) => t,
                Err(e) => {
                    // Mark scanned so we don't repeatedly try a broken blob.
                    self.mark_scanned(crim_path);
                    return Err(format_err!(
                        "failed to parse CRIM/WEVT blob `{}` while scanning templates: {e}",
                        crim_path.display()
                    ));
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

pub(crate) fn normalize_guid(s: &str) -> String {
    s.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}
