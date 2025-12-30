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
//! - `render`: offline rendering helpers for WEVT “inline-name” BinXML (built via the production IR pipeline)
//! - `temp`: helpers for enumerating `TTBL`/`TEMP` entries within a blob (useful for indexing)
//!
//! References:
//! - `docs/wevt_templates.md` (project notes + curated links)
//! - MS-EVEN6 (BinXml inline names + NameHash)
//! - libfwevt manifest format documentation / reference implementation

pub mod manifest;

mod cache;
mod error;
mod extract;
mod render;
mod temp;
mod types;

pub use cache::{WevtCache, WevtCacheError, normalize_guid};
pub use error::WevtTemplateExtractError;
pub use extract::extract_wevt_template_resources;
pub use render::{
    render_temp_to_xml, render_temp_to_xml_with_values, render_template_definition_to_xml,
    render_template_definition_to_xml_with_values,
};
pub use temp::extract_temp_templates_from_wevt_blob;
pub use types::{
    ResourceIdentifier, WevtTempTemplateHeader, WevtTempTemplateRef, WevtTemplateResource,
};
