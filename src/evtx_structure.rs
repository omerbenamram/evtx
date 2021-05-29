use crate::xml_output::BinXmlOutput;
use crate::err::{SerializationError, SerializationResult};
use crate::model::xml::{BinXmlPI, XmlElement, XmlAttribute};
use crate::binxml::value_variant::BinXmlValue;
use crate::binxml::name::BinXmlName;
use std::borrow::Cow;
use chrono::prelude::*;
use std::mem;

mod xml {
  use std::collections::HashMap;

  #[derive(Debug)]
  pub enum XmlContentType {
    Simple(String),
    Complex(Vec<XmlElement>),
    None
  }

  #[derive(Debug)]
  pub struct XmlElement {
    pub name: String,
    pub attributes: HashMap<String, String>,
    pub content: XmlContentType,
  }

  impl XmlElement {
    pub fn new(name: &str) -> Self {
      Self {
        name: name.to_owned(),
        attributes: HashMap::new(),
        content: XmlContentType::None
      }
    }

    pub fn add_attribute(&mut self, name: &str, value: &str) {
      self.attributes.insert(name.to_owned(), value.to_owned());
    }

    pub fn add_simple_content(&mut self, value: &str) {
      match self.content {
        XmlContentType::None => self.content = XmlContentType::Simple(value.to_owned()),
        XmlContentType::Simple(ref mut s) => s.push_str(value),
        _ => panic!("this xml element has already a value assigned: {:?}, trying to add {:?}", self.content, value),
      }
    }
  }

}

pub struct EvtxStructure {
  event_record_id: u64,
  timestamp: DateTime<Utc>,
  content: xml::XmlElement,
}

impl EvtxStructure {
  pub fn new(event_record_id: u64, timestamp: DateTime<Utc>) -> Self {
    Self {
      event_record_id,
      timestamp,
      content: xml::XmlElement::new(""),   // this will be overriden later
    }
  }

  pub fn new_empty() -> Self {
    Self {
      event_record_id: 0,
      timestamp: DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(0, 0), Utc),
      content: xml::XmlElement::new(""),
    }
  }

  pub fn event_record_id(&self) -> u64 {
    self.event_record_id
  }

  pub fn timestamp(&self) -> &DateTime<Utc> {
    &self.timestamp
  }
}

pub struct StructureBuilder {
  result: EvtxStructure,
  node_stack: Vec<xml::XmlElement>,
  unstored_nodes: Vec<xml::XmlElement>,
}

impl StructureBuilder {
  pub fn new(event_record_id: u64, timestamp: DateTime<Utc>) -> Self {
    Self {
      result: EvtxStructure::new(event_record_id, timestamp),
      node_stack: Vec::new(),
      unstored_nodes: Vec::new()
    }
  }

  /// consumes self and returns the generated structure
  pub fn get_structure(&mut self) -> EvtxStructure {
    let mut result = EvtxStructure::new_empty();
    mem::swap(&mut self.result, &mut result);
    return result;
  }

  pub fn enter_named_node(&mut self, name: &str, attributes: &Vec<XmlAttribute>) {
    let mut element = xml::XmlElement::new(name);
    for a in attributes {
      element.add_attribute(a.name.as_ref().as_str(), &a.value.as_ref().as_cow_str());
    }

    self.node_stack.push(element);
  }

  pub fn leave_node(&mut self, _name: &str) {
      match self.node_stack.pop() {
          None => panic!("stack underflow"),
          Some(mut node) => {
            match node.content {

              // this element has no contents, but there are still unstored nodes.
              // we use these as child nodes
              xml::XmlContentType::None => {
                let mut new_nodes = Vec::new();
                mem::swap(&mut self.unstored_nodes, &mut new_nodes);
                node.content = xml::XmlContentType::Complex(new_nodes);
              }

              // this element already has contents, so we cannot add contents to it.
              // we assume this will later be added to its parent
              _ => (),
            }

            self.unstored_nodes.push(node);
          }
      }
  }

  pub fn add_value(&mut self, value: &str) {
    self.node_stack.last_mut().unwrap().add_simple_content(value);
  }
}

impl BinXmlOutput for StructureBuilder {

    /// Called once when EOF is reached.
    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
      if self.node_stack.len() != 0 {
        return Err(SerializationError::StructureBuilderError { message: "node stack is not empty".to_owned() });
      }
      if self.unstored_nodes.len() != 1 {
        return Err(SerializationError::StructureBuilderError { message: "invalid number of unstored nodes".to_owned() });
      }
      self.result.content = self.unstored_nodes.pop().unwrap();
      Ok(())
    }

    /// Called on <Tag attr="value" another_attr="value">.
    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
      let name = element.name.as_ref().as_str();

      self.enter_named_node(name, &element.attributes);
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
    fn visit_entity_reference(&mut self, _: &BinXmlName) -> SerializationResult<()> {
      Ok(())
    }

    /// Emit the characters "&" and "#" and the decimal string representation of the value.
    fn visit_character_reference(&mut self, _: Cow<'_, str>) -> SerializationResult<()> {
      Ok(())
    }

    /// Unimplemented
    fn visit_processing_instruction(&mut self, _: &BinXmlPI) -> SerializationResult<()> {
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