//! Extract WEVT_TEMPLATE resources from PE files.
//!
//! This is primarily intended to support building an offline cache of EVTX templates
//! (see `omerbenamram/evtx` issue #103).

pub mod manifest;

mod binxml;
mod error;
mod extract;
mod render;
mod temp;
mod types;
mod util;

pub use binxml::{parse_temp_binxml_fragment, parse_wevt_binxml_fragment};
pub use error::WevtTemplateExtractError;
pub use extract::extract_wevt_template_resources;
pub use render::{
    render_temp_to_xml, render_template_definition_to_xml,
    render_template_definition_to_xml_with_substitution_values,
};
pub use temp::extract_temp_templates_from_wevt_blob;
pub use types::{
    ResourceIdentifier, WevtTempTemplateHeader, WevtTempTemplateRef, WevtTemplateResource,
};


