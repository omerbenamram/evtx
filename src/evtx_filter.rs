use crate::binxml::value_variant::BinXmlValue;
use crate::binxml::name::BinXmlName;
use crate::err::{SerializationError, SerializationResult};
use crate::model::xml::{BinXmlPI, XmlElement};
use std::borrow::Cow;

#[derive(Clone)]
pub struct EvtxFilter {
    pub ids: Vec<u64>,
}

impl EvtxFilter {
    pub fn empty() -> Self {
        Self {
            ids: Vec::new()
        }
    }

    pub fn new(ids: Vec<u64>) -> Self {
        Self {
            ids: ids
        }
    }

    pub fn matches(&self, record: &std::result::Result<crate::EvtxRecord, crate::err::EvtxError>) -> bool {
        match record {
            Err(_) => false,
            Ok(r) => {
                if ! self.ids.is_empty() {
                    if ! self.ids.contains(&r.event_record_id) {
                        return false;
                    }
                }
                return true;
            }
        }
    }
}

struct FilterableRecord {}

impl crate::BinXmlOutput for FilterableRecord {

    /// Called once when EOF is reached.
    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
        Ok(())
    }

    /// Called on <Tag attr="value" another_attr="value">.
    fn visit_open_start_element(
        &mut self,
        open_start_element: &XmlElement,
    ) -> SerializationResult<()> {
        Ok(())
    }

    /// Called on </Tag>, implementor may want to keep a stack to properly close tags.
    fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        Ok(())
    }

    ///
    /// Called with value on xml text node,  (ex. <Computer>DESKTOP-0QT8017</Computer>)
    ///                                                     ~~~~~~~~~~~~~~~
    fn visit_characters(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
        Ok(())
    }

    /// Unimplemented
    fn visit_cdata_section(&mut self) -> SerializationResult<()> {
        Ok(())
    }

    /// Emit the character "&" and the text.
    fn visit_entity_reference(&mut self, entity: &BinXmlName) -> SerializationResult<()> {
        Ok(())
    }

    /// Emit the characters "&" and "#" and the decimal string representation of the value.
    fn visit_character_reference(&mut self, char_ref: Cow<'_, str>) -> SerializationResult<()> {
        Ok(())
    }

    /// Unimplemented
    fn visit_processing_instruction(&mut self, pi: &BinXmlPI) -> SerializationResult<()> {
        Ok(())
    }

    /// Called once on beginning of parsing.
    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        Ok(())
    }
}
