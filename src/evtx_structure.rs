use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::SerializationResult;
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::xml_output::BinXmlOutput;
use std::borrow::Cow;

/*
 * Generic Parameters:
 * V ... visitor
 * R ... Result of visiting a single record
 */

/// Visitor object which can be used the EvtxStructure shall be printed
pub trait EvtxStructureVisitor {
  type VisitorResult;

  fn get_result(
    &self,
    event_record_id: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
  ) -> Self::VisitorResult;

  /// called when a new record starts
  fn start_record(&mut self) -> SerializationResult<()>;

  /// called when the current records is finished
  fn finalize_record(&mut self) -> SerializationResult<()>;

  // called upon element content
  fn visit_characters(&mut self, value: &str) -> SerializationResult<()>;

  /// called when a complex element (i.e. an element with child elements) starts
  fn visit_start_element<'a, 'b, I>(
    &'a mut self,
    name: &'b str,
    attributes: I,
  ) -> SerializationResult<()>
  where
    'a: 'b,
    I: Iterator<Item = (&'b str, &'b str)> + 'b;

  /// called when a complex element (i.e. an element with child elements) ends
  fn visit_end_element(&mut self, name: &str) -> SerializationResult<()>;
}

pub struct VisitorAdapter<V, R>
where
  V: EvtxStructureVisitor<VisitorResult = R>,
{
  target: V,
}

impl<V, R> VisitorAdapter<V, R>
where
  V: EvtxStructureVisitor<VisitorResult = R>,
{
  pub fn new(target: V) -> Self {
    Self { target }
  }

  pub fn get_result(self, event_record_id: u64, timestamp: chrono::DateTime<chrono::Utc>) -> R {
    self.target.get_result(event_record_id, timestamp)
  }
}
impl<V, R> BinXmlOutput for VisitorAdapter<V, R>
where
  V: EvtxStructureVisitor<VisitorResult = R>,
{
  /// Called once when EOF is reached.
  fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
    self.target.finalize_record()
  }

  /// Called on <Tag attr="value" another_attr="value">.
  fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
    let name = element.name.as_ref().as_str();

    let attributes: Vec<(&str, Cow<'_, str>)> = element
      .attributes
      .iter()
      .map(|a| (a.name.as_ref().as_str(), a.value.as_ref().as_cow_str()))
      .collect();

    self.target.visit_start_element(
      name,
      attributes.iter().map(|(k, v)| (*k, v.as_ref())),
    )
  }

  /// Called on </Tag>, implementor may want to keep a stack to properly close tags.
  fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
    self
      .target
      .visit_end_element(element.name.as_ref().as_str())
  }

  ///
  /// Called with value on xml text node,  (ex. <Computer>DESKTOP-0QT8017</Computer>)
  ///                                                     ~~~~~~~~~~~~~~~
  fn visit_characters(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
    let cow: Cow<str> = value.as_cow_str();
    self.target.visit_characters(&cow)
  }

  /// Unimplemented
  fn visit_cdata_section(&mut self) -> SerializationResult<()> {
    Ok(())
  }

  /// Emit the character "&" and the text.
  fn visit_entity_reference(&mut self, _entity: &BinXmlName) -> SerializationResult<()> {
    Ok(())
  }

  /// Emit the characters "&" and "#" and the decimal string representation of the value.
  fn visit_character_reference(&mut self, _char_ref: Cow<'_, str>) -> SerializationResult<()> {
    Ok(())
  }

  /// Unimplemented
  fn visit_processing_instruction(&mut self, _pi: &BinXmlPI) -> SerializationResult<()> {
    Ok(())
  }

  /// Called once on beginning of parsing.
  fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
    self.target.start_record()
  }
}
