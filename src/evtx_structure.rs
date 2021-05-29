use crate::xml_output::BinXmlOutput;
use crate::err::{SerializationError, SerializationResult};
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::binxml::value_variant::BinXmlValue;
use crate::binxml::name::BinXmlName;
use std::borrow::Cow;
use chrono::prelude::*;
use std::mem;

pub struct EvtxStructure {
  event_record_id: u64,
  timestamp: DateTime<Utc>,
}

impl EvtxStructure {
  pub fn new(event_record_id: u64, timestamp: DateTime<Utc>) -> Self {
    Self {
      event_record_id,
      timestamp
    }
  }

  pub fn add_value(&mut self, path: &Vec<String>, value: &str) {

  }

  pub fn event_record_id(&self) -> u64 {
    self.event_record_id
  }

  pub fn timestamp(&self) -> &DateTime<Utc> {
    &self.timestamp
  }

  pub fn is_ok(&self) -> bool {
    true
  }
}

pub struct StructureBuilder {
  result: Option<EvtxStructure>,
  node_stack: Vec<String>,
}

impl StructureBuilder {
  pub fn new(event_record_id: u64, timestamp: DateTime<Utc>) -> Self {
    Self {
      result: Some(EvtxStructure::new(event_record_id, timestamp)),
      node_stack: Vec::new()
    }
  }

  /// consumes self and returns the generated structure
  pub fn get_structure(&mut self) -> EvtxStructure {
    let mut result = None;
    mem::swap(&mut self.result, &mut result);
    return result.unwrap();
  }

  pub fn enter_named_node(&mut self, name: &str) {
    self.node_stack.push(String::from(name));
  }

  pub fn leave_node(&mut self, _name: &str) {
      match self.node_stack.pop() {
          None => panic!("stack underflow"),
          _ => ()
      }
  }

  pub fn add_value(&mut self, value: &str) {
    if let Some(result) = &mut self.result {
      result.add_value(&self.node_stack, value);
    }
  }
}

impl BinXmlOutput for StructureBuilder {

    /// Called once when EOF is reached.
    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
      if self.node_stack.len() != 0 {
        return Err(SerializationError::StructureBuilderError { message: "node stack is not empty".to_owned() });
      }
      Ok(())
    }

    /// Called on <Tag attr="value" another_attr="value">.
    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
      let name = element.name.as_ref().as_str();
      self.enter_named_node(name);
      Ok(())
    }

    /// Called on </Tag>, implementor may want to keep a stack to properly close tags.
    fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
      let name = element.name.as_ref().as_str();
      self.leave_node(&name);
      Ok(())
    }

    ///
    /// Called with value on xml text node,  (ex. <Computer>DESKTOP-0QT8017</Computer>)
    ///                                                     ~~~~~~~~~~~~~~~
    fn visit_characters(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
      let cow: Cow<str> = value.as_cow_str();
      self.add_value(&cow);
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
      if self.node_stack.len() != 0 {
        panic!("internal error: node stack is not empty");
      }
      Ok(())
    }
}