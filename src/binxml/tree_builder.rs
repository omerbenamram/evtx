use binxml::model::BinXMLTemplate;
use binxml::model::BinXMLTemplateDefinition;
use binxml::model::BinXMLValue;
use binxml::model::OpenStartElementTokenMeta;
use binxml::model::{
    BinXMLAttribute, BinXMLDeserializedTokens, BinXMLFragmentHeader, BinXMLOpenStartElement,
};
use binxml::owned_model::Element;
use hexdump::print_hexdump;
use indextree::{Arena, NodeId};
use std::io::{Cursor, Read, Result, Seek, SeekFrom};
use std::marker::PhantomData;
use std::borrow::BorrowMut;
use num_traits::Num;

pub trait Visitor<'a> {
    fn visit_end_of_stream(&mut self) -> ();
    fn visit_open_start_element(&mut self, open_start_element: &'a BinXMLOpenStartElement) -> ();
    fn visit_close_start_element(&mut self) -> ();
    fn visit_close_empty_element(&mut self) -> ();
    fn visit_close_element(&mut self) -> ();
    fn visit_value(&mut self, value: &'a BinXMLValue<'a>) -> ();
    fn visit_attribute(&mut self, attribute: &'a BinXMLAttribute) -> ();
    fn visit_cdata_section(&mut self) -> ();
    fn visit_entity_reference(&mut self) -> ();
    fn visit_processing_instruction_target(&mut self) -> ();
    fn visit_processing_instruction_data(&mut self) -> ();
    fn visit_normal_substitution(&mut self) -> ();
    fn visit_conditional_substitution(&mut self) -> ();
    fn visit_template_instance(&mut self, template: &'a BinXMLTemplate) -> ();
    fn visit_start_of_stream(&mut self, header: &'a BinXMLFragmentHeader) -> ();
}

#[derive(Debug)]
struct BinXMLTreeBuilder<'a, N: Num> {
    template: Option<&'a BinXMLTemplateDefinition<'a>>,
    xml: ElementTree<N>,
    current_parent: Option<NodeId>,
}

impl<'a, N: Num> BinXMLTreeBuilder<'a, N> {
    fn add_leaf(&mut self, node: NodeId) -> () {
        self.current_parent.unwrap().append(node, &mut self.xml);
    }

    fn add_node(&mut self, node: NodeId) -> () {
        match self.current_parent {
            Some(parent) => {
                parent.append(node, &mut self.xml);
                self.current_parent = Some(node);
            }
            None => self.current_parent = Some(node),
        }
    }
}

impl<'a, N: Num> Visitor<'a> for BinXMLTreeBuilder<'a, N> {
    fn visit_end_of_stream(&mut self) {
        println!("visit_end_of_stream");
    }

    fn visit_open_start_element(&mut self, tag: &'a BinXMLOpenStartElement) {
        debug!("visit start_element {:?}", tag);
//        let node = self
//            .xml
//            .new_node(BinXMLDeserializedTokens::OpenStartElement(tag.clone()));
//        self.add_node(node);
    }

    fn visit_close_start_element(&mut self) {
        println!("visit_close_start_element");
        let node = self.current_parent.unwrap();
        let parent = self.xml.get(node).unwrap().parent();
        self.current_parent = parent;
    }

    fn visit_close_empty_element(&mut self) {
        println!("visit_close_empty_element");
        unimplemented!();
    }

    fn visit_close_element(&mut self) {
        println!("visit_close_element");
        unimplemented!();
    }

    fn visit_value(&mut self, value: &'a BinXMLValue<'a>) -> () {
        debug!("visit_value");
//        let node = self
//            .xml
//            .new_node(BinXMLDeserializedTokens::Value(value.clone()));
//        self.add_leaf(node);
    }

    fn visit_attribute(&mut self, attribute: &'a BinXMLAttribute) -> () {
        unimplemented!()
    }

    fn visit_cdata_section(&mut self) {
        println!("visit_cdata_section");
        unimplemented!();
    }

    fn visit_entity_reference(&mut self) {
        println!("visit_entity_reference");
        unimplemented!();
    }

    fn visit_processing_instruction_target(&mut self) {
        println!("visit_processing_instruction_target");
        unimplemented!();
    }

    fn visit_processing_instruction_data(&mut self) {
        println!("visit_processing_instruction_data");
        unimplemented!();
    }

    fn visit_normal_substitution(&mut self) {
        println!("visit_normal_substitution");
        unimplemented!();
    }

    fn visit_conditional_substitution(&mut self) {
        println!("visit_conditional_substitution");
        unimplemented!();
    }

    fn visit_template_instance(&mut self, template: &'a BinXMLTemplate) -> () {
        let elem_unfilled = &template.definition.element;
        let mut elem_filled = elem_unfilled.clone();
        let substitutions = &template.substitution_array;

        for (i, sub_elem) in elem_filled.iter_mut().enumerate() {
            if let BinXMLDeserializedTokens::Substitution(ref data) = sub_elem {
                *sub_elem = BinXMLDeserializedTokens::Value(
                    substitutions[data.substitution_index as usize].clone(),
                )
            }
        }
    }

    fn visit_start_of_stream(&mut self, header: &'a BinXMLFragmentHeader) -> () {
        debug!("visit_start_of_stream");
    }
}

pub type ElementTree<N> = Arena<Element<N>>;
