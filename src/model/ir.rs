//! Intermediate representation (IR) for parsed BinXML content.
//!
//! This module defines a small, allocation-friendly tree that mirrors the
//! structure of BinXML records while keeping names and text separate. The IR
//! is used by renderers (e.g. JSON streaming) and by template instantiation.
//!
//! Design notes:
//! - Names and text are borrowed when possible via `Cow` to avoid copies.
//! - `Node::Placeholder` is used only inside cached template definitions and
//!   must be resolved before rendering.
//! - `Element::has_element_child` is maintained to optimize rendering decisions.

use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType};
use std::borrow::Cow;

/// An XML name backed by a BinXML name entry.
///
/// Names are guaranteed to be valid XML names as produced by the BinXML
/// stream. The underlying value may be borrowed from the chunk string table
/// or owned when decoded inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name<'a> {
    value: Cow<'a, BinXmlName>,
}

impl<'a> Name<'a> {
    /// Wrap a `BinXmlName` as an IR name.
    pub fn new(value: Cow<'a, BinXmlName>) -> Self {
        Name { value }
    }

    /// Returns the underlying `BinXmlName`.
    pub fn as_binxml_name(&self) -> &BinXmlName {
        self.value.as_ref()
    }

    /// Returns the name as a UTF-8 string slice.
    pub fn as_str(&self) -> &str {
        self.value.as_ref().as_str()
    }
}

/// Text content stored as UTF-8.
///
/// Text is decoded at parse time and may be borrowed from the input buffer
/// when possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text<'a> {
    pub value: Cow<'a, str>,
}

impl<'a> Text<'a> {
    /// Wrap a text value as an IR text node.
    pub fn new(value: Cow<'a, str>) -> Self {
        Text { value }
    }
}

/// A single node in the IR tree.
///
/// `Placeholder` nodes only appear in cached template definitions. Renderers
/// should never see unresolved placeholders.
#[derive(Debug, Clone, PartialEq)]
pub enum Node<'a> {
    Element(Box<Element<'a>>),
    Text(Text<'a>),
    Value(BinXmlValue<'a>),
    EntityRef(Name<'a>),
    CharRef(u16),
    CData(Text<'a>),
    PITarget(Name<'a>),
    PIData(Text<'a>),
    Placeholder(Placeholder),
}

/// Template substitution placeholder captured during template parsing.
///
/// `id` indexes into the template substitution array. `value_type` is the
/// declared substitution type, and `optional` indicates the substitution may
/// be omitted if empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    pub id: u16,
    pub value_type: BinXmlValueType,
    pub optional: bool,
}

/// An attribute name plus its value nodes.
///
/// Attribute values are stored as a sequence of non-element nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr<'a> {
    pub name: Name<'a>,
    pub value: Vec<Node<'a>>,
}

/// An element with attributes and child nodes.
///
/// `has_element_child` is tracked to speed up JSON rendering decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct Element<'a> {
    pub name: Name<'a>,
    pub attrs: Vec<Attr<'a>>,
    pub children: Vec<Node<'a>>,
    pub has_element_child: bool,
}

impl<'a> Element<'a> {
    /// Create a new element with the provided name.
    pub fn new(name: Name<'a>) -> Self {
        Element {
            name,
            attrs: Vec::new(),
            children: Vec::new(),
            has_element_child: false,
        }
    }

    /// Append a child node and update `has_element_child` if needed.
    pub fn push_child(&mut self, node: Node<'a>) {
        if matches!(node, Node::Element(_)) {
            self.has_element_child = true;
        }
        self.children.push(node);
    }
}
