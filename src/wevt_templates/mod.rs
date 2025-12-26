//! Offline extraction/parsing/rendering of Windows Event Log templates (`WEVT_TEMPLATE`).
//!
//! EVTX records often contain *template instances* (substitution values), while the corresponding
//! *template definitions* are stored in provider PE resources under the `WEVT_TEMPLATE` type.
//! This module provides the pieces needed to build an offline cache and render events without
//! calling Windows APIs.
//!
//! The implementation is split into a few focused submodules:
//! - `extract`: minimal, bounds-checked PE/RSRC parsing to extract `WEVT_TEMPLATE` blobs
//! - `manifest`: spec-backed parsing of the CRIM/WEVT payload, plus stable join keys
//! - `binxml` + `render`: decoding/rendering of the WEVT “inline-name” BinXML dialect
//! - `temp`: helpers for enumerating `TTBL`/`TEMP` entries within a blob (useful for indexing)
//!
//! References:
//! - `docs/wevt_templates.md` (project notes + curated links)
//! - MS-EVEN6 (BinXml inline names + NameHash)
//! - libfwevt manifest format documentation / reference implementation

pub mod manifest;

mod binxml;
mod cache;
mod error;
mod extract;
mod record_fallback;
mod render;
mod temp;
mod types;

pub use binxml::{parse_temp_binxml_fragment, parse_wevt_binxml_fragment};
pub use cache::{WevtCache, WevtCacheError, normalize_guid};
pub use error::WevtTemplateExtractError;
pub use extract::extract_wevt_template_resources;
pub use render::{
    render_temp_to_xml, render_temp_to_xml_with_substitution_values,
    render_template_definition_to_xml, render_template_definition_to_xml_with_substitution_values,
};
pub use temp::extract_temp_templates_from_wevt_blob;
pub use types::{
    ResourceIdentifier, WevtTempTemplateHeader, WevtTempTemplateRef, WevtTemplateResource,
};
