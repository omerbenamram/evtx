use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::SerializationResult;
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::xml_output::BinXmlOutput;
use std::borrow::Cow;
use std::marker::PhantomData;

pub trait VisitorBuilder<R>: Send + Sync + Clone + Sized {
  fn build(&self) -> Box<dyn EvtxStructureVisitor<VisitorResult=R>>;
}

/// Visitor object which can be used the EvtxStructure shall be printed
pub trait EvtxStructureVisitor {
  type VisitorResult;

  fn get_result(&self) -> Self::VisitorResult;

  /// called when a new record starts
  fn start_record(&mut self);

  /// called when the current records is finished
  fn finalize_record(&mut self);

  // called upon element content
  fn visit_characters(&mut self, value: &str);  

  /// called on any structure element with a content type of `None`
  fn visit_empty_element<'a, 'b>(&'a mut self, name: &'b str, attributes: Box<dyn Iterator<Item=(&'b str, &'b str)> + 'b>) where 'a: 'b;

  /// called on any structure element which contains only a textual value
  fn visit_simple_element<'a, 'b>(&'a mut self, name: &'b str, attributes: Box<dyn Iterator<Item=(&'b str, &'b str)> + 'b>, content: &'b str) where 'a: 'b;

  /// called when a complex element (i.e. an element with child elements) starts
  fn visit_start_element<'a, 'b>(&'a mut self, name: &'b str, attributes: Box<dyn Iterator<Item=(&'b str, &'b str)> + 'b>) where 'a: 'b;

  /// called when a complex element (i.e. an element with child elements) ends
  fn visit_end_element(&mut self, name: &str);
}

pub struct VisitorAdapter<C, R> where C: VisitorBuilder<R> {
  target: Box<dyn EvtxStructureVisitor<VisitorResult=R>>,
  phantom_c: PhantomData<C>,
  phantom_r: PhantomData<R>,
}

impl<C, R> VisitorAdapter<C, R> where C: VisitorBuilder<R> {
  pub fn new(builder: &C) -> Self {
    Self {
      target: builder.build(),
      phantom_c: PhantomData,
      phantom_r: PhantomData
    }
  }

  pub fn get_result(self) -> Box<R> {
    Box::new(self.target.get_result())
  }
}
impl<C, R> BinXmlOutput for VisitorAdapter<C, R> where C: VisitorBuilder<R> {
  /// Called once when EOF is reached.
  fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
    self.target.finalize_record();
    Ok(())
  }

  /// Called on <Tag attr="value" another_attr="value">.
  fn visit_open_start_element(
      &mut self,
      element: &XmlElement,
  ) -> SerializationResult<()> {
    let name = element.name.as_ref().as_str();

    let attributes: Vec<(&str, Cow<'_, str>)> = element.attributes.iter().map(|a| (a.name.as_ref().as_str(), a.value.as_ref().as_cow_str())).collect();

    self.target.visit_start_element(
      name,
      Box::new(attributes.iter().map(|(k,v)| (*k, v.as_ref())))
    );
    Ok(())
  }

  /// Called on </Tag>, implementor may want to keep a stack to properly close tags.
  fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
    self.target.visit_end_element(element.name.as_ref().as_str());
    Ok(())
  }

  ///
  /// Called with value on xml text node,  (ex. <Computer>DESKTOP-0QT8017</Computer>)
  ///                                                     ~~~~~~~~~~~~~~~
  fn visit_characters(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
    let cow: Cow<str> = value.as_cow_str();
    self.target.visit_characters(&cow);
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
    self.target.start_record();
    Ok(())
  }
}