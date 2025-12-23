//! Parsing for the `WEVT_TEMPLATE` resource payload (CRIM/WEVT/...).
//!
//! This module is a Rust port of the libyal/libfwevt "Windows Event manifest binary format"
//! documentation and aligns with the reference C implementation.
//!
//! Primary references:
//! - libfwevt: `documentation/Windows Event manifest binary format.asciidoc`
//! - MS-EVEN6: BinXml name hashing/layout and token grammar
//!
//! Design goals:
//! - Deterministic parsing (no signature scanning).
//! - Strict bounds/sanity checks; offsets are validated relative to the CRIM blob.
//! - Preserve unknown fields as raw integers/bytes (do not guess semantics).
//! - Provide stable join keys: provider GUID + event (id/version/...) + template offset.
//!
//! Note: libfwevt's map parsing is marked TODO; we parse VMAP per spec and keep unknown map
//! types as raw bytes.
//!
//! This module is split into:
//! - `types`: a typed view of the manifest structures (kept stable for downstream join/render code)
//! - `parse`: spec-backed parsing and bounds validation
//! - `error`: a small error enum that makes failures actionable in tests/tooling
//!
//! References:
//! - `docs/wevt_templates.md` (project notes + curated links)
//! - libfwevt manifest spec doc (CRIM/WEVT/EVNT/TTBL/TEMP)
//! - MS-EVEN6 (BinXml grammar notes used by template rendering)

mod error;
mod parse;
mod types;

pub use error::WevtManifestError;
pub use types::*;


