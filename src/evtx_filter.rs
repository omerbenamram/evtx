use crate::binxml::value_variant::BinXmlValue;
use crate::binxml::name::BinXmlName;
use crate::binxml::assemble::parse_tokens;
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
        // if nobody entered some filter conditions, every record matches
        if self.ids.len() == 0 {
            return true
        }

        match record {
            Err(_) => false,
            Ok(r) => self.matches_record(r)
        }
    }

    fn matches_record<'a> (&'a self, record: &crate::EvtxRecord) -> bool {
        let mut builder = RecordVisitor::new(self);
        match parse_tokens(record.tokens.clone(), &record.chunk, &mut builder) {
            Err(_) => return false,
            Ok(_) => builder.matches_filter()
        }
    }

    pub fn match_value(&self, _path: &[String], _value: &Cow<str>) -> bool {
        false
    }

    pub fn match_eventid(&self, eventid: u64) -> bool {
        self.ids.contains(&eventid)
    }
}

struct RecordVisitor<'a> {
    matches_filter: bool,
    node_stack: Vec<String>,
    filter: &'a EvtxFilter
}

impl<'a> RecordVisitor<'a> {
    pub fn new(filter: &'a EvtxFilter) -> Self {
        Self {
            matches_filter: false,
            node_stack: Vec::new(),
            filter
        }
    }

    pub fn matches_filter(&self) -> bool {
        self.matches_filter
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
}

impl<'a> crate::BinXmlOutput for RecordVisitor<'a> {

    /// Called once when EOF is reached.
    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
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
        if self.matches_filter {
            return Ok(())
        }

        let cow: Cow<str> = value.as_cow_str();
        if self.node_stack.len() == 3 && 
            self.node_stack[0] == "Event" &&
            self.node_stack[1] == "System" &&
            self.node_stack[2] == "EventID" {
            match cow.parse::<u64>() {
                Err(e) => return Err(SerializationError::ParseIntError{source: e}),
                Ok(eventid) => {
                    self.matches_filter = self.filter.match_eventid(eventid);
                    return Ok(())
                }
            }
        }
        
        self.matches_filter = self.filter.match_value(&self.node_stack[..], &cow);
        Ok(())
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
        Ok(())
    }
}
