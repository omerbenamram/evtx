use binxml::model::BinXMLTemplate;
use binxml::model::OpenStartElementToken;
use binxml::model::{
    BinXMLAttribute, BinXMLFragmentHeader, BinXMLOpenStartElement, BinXMLParsedNodes,
    BinXMLValueText,
};
use hexdump::print_hexdump;
use indextree::{Arena, NodeId};
use std::io::{Cursor, Read, Result, Seek, SeekFrom};
use std::marker::PhantomData;

pub trait Visitor<'a> {
    fn visit_end_of_stream(&mut self) -> ();
    fn visit_open_start_element(&mut self, open_start_element: &'a BinXMLOpenStartElement) -> ();
    fn visit_close_start_element(&mut self) -> ();
    fn visit_close_empty_element(&mut self) -> ();
    fn visit_close_element(&mut self) -> ();
    // TODO: fix value type! (needs to be enum)
    fn visit_value(&mut self, value: &'a BinXMLValueText) -> ();
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
struct BinXMLVisitor<'a> {
    template: Option<&'a BinXMLTemplate>,
    xml: Arena<BinXMLParsedNodes>,
    current_parent: Option<NodeId>,
}

impl<'a> BinXMLVisitor<'a> {
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

impl<'a> Visitor<'a> for BinXMLVisitor<'a> {
    fn visit_end_of_stream(&mut self) {
        println!("visit_end_of_stream");
    }

    fn visit_open_start_element(&mut self, tag: &'a BinXMLOpenStartElement) {
        debug!("visit start_element {:?}", tag);
        let node = self
            .xml
            .new_node(BinXMLParsedNodes::OpenStartElement(tag.clone()));
        self.add_node(node);
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

    fn visit_value(&mut self, value: &'a BinXMLValueText) -> () {
        debug!("visit_value");
        let node = self
            .xml
            .new_node(BinXMLParsedNodes::ValueText(value.clone()));
        self.add_leaf(node);
    }

    fn visit_attribute(&mut self, attribute: &'a BinXMLAttribute) -> () {
        debug!("visit_attribute");
        let node = self
            .xml
            .new_node(BinXMLParsedNodes::Attribute(attribute.clone()));
        self.add_leaf(node);
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
        debug!("visit_template_instance");
        self.template = Some(template);
    }

    fn visit_start_of_stream(&mut self, header: &'a BinXMLFragmentHeader) -> () {
        debug!("visit_start_of_stream");
        let node = self
            .xml
            .new_node(BinXMLParsedNodes::FragmentHeader(header.clone()));
        self.add_node(node);
    }
}

type BinXML = Arena<BinXMLParsedNodes>;

//fn parse_binxml<'a, V: Visitor<'a>>(data: &[u8], offset: u64, visitor: &mut V) -> BinXML {
//    let mut ctx = BinXMLVisitor::new(data, offset);
//
//    //    dump(&mut ctx, 0);
//    // TODO: actually break
//    for token in 0..10 {}
//}
