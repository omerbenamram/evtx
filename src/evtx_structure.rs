use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{SerializationError, SerializationResult};
use crate::model::xml::{BinXmlPI, XmlAttribute, XmlElement};
use crate::xml_output::BinXmlOutput;
use chrono::prelude::*;
use chrono::format::ParseResult;
use std::borrow::Cow;
use std::mem;
use std::cmp::{Ord, Ordering};
use std::collections::HashMap;

#[derive(Debug)]
enum EvtxXmlContentType {
  Simple(String),
  Complex(Vec<EvtxXmlElement>),
  None,
}

#[derive(Debug)]
struct EvtxXmlElement {
  pub name: String,
  pub attributes: HashMap<String, String>,
  pub content: EvtxXmlContentType,
}

impl EvtxXmlElement {
  pub fn new(name: &str) -> Self {
    Self {
      name: name.to_owned(),
      attributes: HashMap::new(),
      content: EvtxXmlContentType::None,
    }
  }

  pub fn add_attribute(&mut self, name: &str, value: &str) {
    self.attributes.insert(name.to_owned(), value.to_owned());
  }

  pub fn add_simple_content(&mut self, value: &str) {
    match self.content {
      EvtxXmlContentType::None => self.content = EvtxXmlContentType::Simple(value.to_owned()),
      EvtxXmlContentType::Simple(ref mut s) => s.push_str(value),
      _ => {
        if !value.is_empty() {
          panic!(
            "this xml element has already a value assigned: {:?}, trying to add {:?}",
            self.content, value
          )
        }
      }
    }
  }

  pub fn add_child(&mut self, child: EvtxXmlElement) {
    match self.content {
      EvtxXmlContentType::Simple(_) => {
        panic!("this xml element is a text node and cannot contain child elements")
      }
      EvtxXmlContentType::None => self.content = EvtxXmlContentType::Complex(vec![child]),
      EvtxXmlContentType::Complex(ref mut v) => v.push(child),
    }
  }
}

pub struct EvtxStructure {
  event_record_id: u64,
  timestamp: DateTime<Utc>,
  content: EvtxXmlElement,
}

impl EvtxStructure {
  pub fn new(event_record_id: u64, timestamp: DateTime<Utc>) -> Self {
    Self {
      event_record_id,
      timestamp,
      content: EvtxXmlElement::new(""), // this will be overriden later
    }
  }

  pub fn new_empty() -> Self {
    Self {
      event_record_id: 0,
      timestamp: DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(0, 0), Utc),
      content: EvtxXmlElement::new(""),
    }
  }

  /// Returns the current record ID. Beware: This is *not* the event ID!
  /// If you need the event id, call `event_id()`.
  pub fn event_record_id(&self) -> u64 {
    self.event_record_id
  }

  /// Returns the timestamp of the record structure
  pub fn timestamp(&self) -> &DateTime<Utc> {
    &self.timestamp
  }

  /// Returns the event id
  pub fn event_id(&self) -> u32 {
    self.find(&["System", "EventID"])
    .expect("missing EventID")
    .parse()
    .expect("invalid EventID")
  }

  /// returns TimeCreated/@SystemTime
  pub fn time_created(&self) -> ParseResult<DateTime<Utc>> {
    let dt = self.find(&["System", "TimeCreated", "@SystemTime"])
    .expect("missing TimeCreated");
    match NaiveDateTime::parse_from_str(dt, "%F %T%.f %Z") {
      Ok(dt) => Ok(DateTime::from_utc(dt, Utc)),
      Err(e) => Err(e)
    }
  }

  /// returns System/Provider/@Name
  pub fn provider_name(&self) -> &str {
    self.find(&["System", "Provider", "@Name"]).expect("missing Provider name")
  }

  /// Find a single value of the current event record.
  /// 
  /// The path to the required value must be specified by using an XPath-like
  /// syntax, but not as a single String, but as an array of path components.
  /// For example, `/System/TimeCreated/@SystemTime` must be specified as 
  /// `&["System", "TimeCreated", "@SystemTime"]`.
  /// 
  /// # Example
  /// ```
  /// # use evtx::EvtxParser;
  /// # use std::path::PathBuf;
  /// # pub fn samples_dir() -> PathBuf {
  /// #  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
  /// #  .join("samples")
  /// #  .canonicalize()
  /// #  .unwrap()
  /// # }
  /// #
  /// # pub fn regular_sample() -> PathBuf {
  /// # samples_dir().join("security.evtx")
  /// # }
  /// # let mut parser = EvtxParser::from_path(regular_sample()).unwrap();
  /// for record_res in parser.records_struct() {
  ///   match record_res {
  ///     Ok(record) => {
  ///       let event_id = record.find(&["System", "EventID"]).unwrap();
  ///       let time_created = record.find(&["System", "TimeCreated", "@SystemTime"]);
  ///       println!("{}", event_id);
  ///     }
  ///     _ => eprintln!("error"),
  ///   }
  /// }
  /// ```
  pub fn find(&self, path: &[&str]) -> Option<&str> {
    self.find_r(&self.content, path)
  }

  fn find_r<'a>(&'a self, root: &'a EvtxXmlElement, path: &[&str]) -> Option<&'a str> {
    if path.len() == 1 {
      if path[0].chars().next().unwrap() == '@' {
        let attribute_name = &path[0][1..];
        match root.attributes.get(attribute_name) {
          Some(ref v) => return Some(v),
          None        => return None,
        }
      }
    }

    match root.content {
      EvtxXmlContentType::None => panic!("invalid node type"),
      EvtxXmlContentType::Simple(ref s) => 
        if path.is_empty() { Some(s) } else { log::error!("path is NOT empty"); None },
      EvtxXmlContentType::Complex(ref c) => {
        if path.is_empty() {
          log::error!("path IS empty");
          None
        } else {
          let next_name = &path[0];
          let remaining = &path[1..];
          match c.iter().find(|&e| &e.name == next_name) {
            None => { 
              let names: Vec<&String> = c.iter().map(|r| &r.name).collect();
              log::error!("did not find child in {:?}", names); 
              None},
            Some(ref next_node) => self.find_r(next_node, remaining)
          }
        }
      }
    }
  }
}

impl PartialEq for EvtxStructure {
  fn eq(&self, other: &Self) -> bool {
    self.event_record_id() == other.event_record_id()
  }
}
impl Eq for EvtxStructure {}

impl Ord for EvtxStructure {
  fn cmp(&self, other: &Self) -> Ordering {
    self.event_record_id.cmp(&other.event_record_id)
  }
}

impl PartialOrd for EvtxStructure {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

pub struct StructureBuilder {
  result: EvtxStructure,
  node_stack: Vec<EvtxXmlElement>,
}

impl StructureBuilder {
  pub fn new(event_record_id: u64, timestamp: DateTime<Utc>) -> Self {
    Self {
      result: EvtxStructure::new(event_record_id, timestamp),
      node_stack: Vec::new(),
    }
  }

  /// consumes self and returns the generated structure
  pub fn get_structure(&mut self) -> EvtxStructure {
    let mut result = EvtxStructure::new_empty();
    mem::swap(&mut self.result, &mut result);
    return result;
  }

  pub fn enter_named_node(&mut self, name: &str, attributes: &Vec<XmlAttribute>) {
    let mut element = EvtxXmlElement::new(name);
    for a in attributes {
      element.add_attribute(a.name.as_ref().as_str(), &a.value.as_ref().as_cow_str());
    }
    self.node_stack.push(element);
  }

  pub fn leave_node(&mut self, _name: &str) {
    let my_node = self.node_stack.pop().expect("stack underflow");
    if self.node_stack.is_empty() {
      self.result.content = my_node;
    } else {
      self.node_stack.last_mut().unwrap().add_child(my_node);
    }
  }
}

impl BinXmlOutput for StructureBuilder {
  /// Called once when EOF is reached.
  fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
    if !self.node_stack.is_empty() {
      return Err(SerializationError::StructureBuilderError {
        message: "node stack is not empty".to_owned(),
      });
    }
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
    self.node_stack.last_mut().unwrap().add_simple_content(&cow);
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
