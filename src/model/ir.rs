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
use indextree::{Arena, NodeId};
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
/// UTF-8 text is used for both decoded BinXML strings and synthetic values
/// (e.g. WEVT substitutions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text<'a> {
    /// UTF-8 text, borrowed or owned.
    Utf8(Cow<'a, str>),
}

impl<'a> Text<'a> {
    /// Wrap UTF-8 text as an IR text node.
    pub fn new(value: Cow<'a, str>) -> Self {
        Text::Utf8(value)
    }

    /// Wrap UTF-8 text as an IR text node.
    pub fn utf8(value: Cow<'a, str>) -> Self {
        Text::Utf8(value)
    }

    /// Returns true if the text is empty.
    pub fn is_empty(&self) -> bool {
        match self {
            Text::Utf8(value) => value.is_empty(),
        }
    }

    /// Returns a UTF-8 view when this text is already UTF-8.
    pub fn as_utf8(&self) -> Option<&str> {
        match self {
            Text::Utf8(value) => Some(value.as_ref()),
        }
    }
}

/// A single node in the IR tree.
///
/// `Placeholder` nodes only appear in cached template definitions. Renderers
/// should never see unresolved placeholders.
#[derive(Debug, Clone, PartialEq)]
pub enum Node<'a> {
    /// Reference to an element stored in the IR arena.
    Element(NodeId),
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

/// Identifier for an element stored in an IR arena.
pub type ElementId = NodeId;

/// Arena-backed IR tree.
///
/// The tree owns an `indextree::Arena` of elements and stores the root node ID.
/// All element references inside `Node::Element` variants point back into this
/// arena.
#[derive(Debug, Clone)]
pub struct IrTree<'a> {
    arena: Arena<Element<'a>>,
    root: ElementId,
}

impl<'a> IrTree<'a> {
    /// Create a new IR tree from the provided arena and root ID.
    pub fn new(arena: Arena<Element<'a>>, root: ElementId) -> Self {
        IrTree { arena, root }
    }

    /// Returns the root element ID.
    pub fn root(&self) -> ElementId {
        self.root
    }

    /// Returns a shared reference to the element arena.
    pub fn arena(&self) -> &Arena<Element<'a>> {
        &self.arena
    }

    /// Returns a mutable reference to the element arena.
    pub fn arena_mut(&mut self) -> &mut Arena<Element<'a>> {
        &mut self.arena
    }

    /// Returns the root element.
    pub fn root_element(&self) -> &Element<'a> {
        self.element(self.root)
    }

    /// Returns a reference to the element for the given ID.
    pub fn element(&self, id: ElementId) -> &Element<'a> {
        self.arena
            .get(id)
            .expect("invalid element id")
            .get()
    }

    /// Returns a mutable reference to the element for the given ID.
    pub fn element_mut(&mut self, id: ElementId) -> &mut Element<'a> {
        self.arena
            .get_mut(id)
            .expect("invalid element id")
            .get_mut()
    }
}
