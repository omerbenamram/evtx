//! Intermediate representation (IR) for parsed BinXML content.
//!
//! This module defines a small, allocation-friendly tree that mirrors the
//! structure of BinXML records while keeping names and text separate. The IR
//! is used by renderers (e.g. JSON streaming) and by template instantiation.
//!
//! Design notes:
//! - Names and text are borrowed when possible via `Cow` to avoid copies.
//! - `Node::Placeholder` is used only inside cached template definitions and
//!   is resolved during template instantiation (IR build), before rendering.
//! - `Element::has_element_child` is maintained to optimize rendering decisions.

use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType};
use crate::utils::Utf16LeSlice;
use bumpalo::Bump;
use bumpalo::collections::Vec as BumpVec;
use std::mem::ManuallyDrop;

/// An XML name backed by a UTF-8 string slice.
///
/// Names are guaranteed to be valid XML names as produced by the BinXML
/// stream.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Name<'a> {
    value: &'a str,
}

impl<'a> Name<'a> {
    /// Wrap a string slice as an IR name.
    pub fn new(value: &'a str) -> Self {
        Name { value }
    }

    /// Returns the name as a UTF-8 string slice.
    pub fn as_str(&self) -> &str {
        self.value
    }
}

/// Text content stored as UTF-16LE or UTF-8.
///
/// BinXML text is preserved in UTF-16LE to avoid eager decoding and to enable
/// fast SIMD escaping when rendering JSON/XML. UTF-8 text is reserved for
/// synthetic or already-decoded content (e.g. ANSI strings or template
/// substitutions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text<'a> {
    /// UTF-16LE text slice borrowed from the chunk.
    Utf16(Utf16LeSlice<'a>),
    /// UTF-8 text (borrowed from chunk/bump storage).
    Utf8(&'a str),
}

impl<'a> Text<'a> {
    /// Wrap UTF-16LE text as an IR text node.
    pub fn utf16(value: Utf16LeSlice<'a>) -> Self {
        Text::Utf16(value)
    }

    /// Wrap UTF-8 text as an IR text node.
    pub fn utf8(value: &'a str) -> Self {
        Text::Utf8(value)
    }

    /// Returns true if the text is empty.
    pub fn is_empty(&self) -> bool {
        match self {
            Text::Utf16(value) => value.is_empty(),
            Text::Utf8(value) => value.is_empty(),
        }
    }

    /// Returns a UTF-8 view when this text is already UTF-8.
    pub fn as_utf8(&self) -> Option<&str> {
        match self {
            Text::Utf16(_) => None,
            Text::Utf8(value) => Some(value),
        }
    }

    /// Returns the UTF-16LE slice when this text is stored as UTF-16LE.
    pub fn as_utf16(&self) -> Option<Utf16LeSlice<'_>> {
        match self {
            Text::Utf16(value) => Some(*value),
            Text::Utf8(_) => None,
        }
    }
}

/// A single node in the IR tree.
///
/// `Placeholder` nodes only appear in cached template definitions. Renderers
/// should resolve them when rendering a `TemplateInstance`.
#[derive(Debug, Clone, PartialEq)]
pub enum Node<'a> {
    /// Reference to an element stored in the IR arena.
    Element(ElementId),
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
    pub value: IrVec<'a, Node<'a>>,
}

/// Substitution values captured for a template instance.
///
/// `Value` stores the raw BinXML value, while `BinXmlElement` references a
/// pre-parsed BinXML fragment that was expanded into the record arena.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateValue<'a> {
    /// A raw substitution value.
    Value(BinXmlValue<'a>),
    /// A parsed BinXML fragment stored in the record arena.
    BinXmlElement(ElementId),
}

/// An element with attributes and child nodes.
///
/// `has_element_child` is tracked to speed up JSON rendering decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct Element<'a> {
    pub name: Name<'a>,
    pub attrs: IrVec<'a, Attr<'a>>,
    pub children: IrVec<'a, Node<'a>>,
    pub has_element_child: bool,
}

impl<'a> Element<'a> {
    /// Create a new element with the provided name, allocating vectors in the bump arena.
    pub fn new_in(name: Name<'a>, arena: &'a Bump) -> Self {
        Element {
            name,
            attrs: IrVec::new_in(arena),
            children: IrVec::new_in(arena),
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

/// Bump-allocated vector type used inside IR nodes.
pub type IrVec<'a, T> = BumpVec<'a, T>;

/// Identifier for an element stored in an IR arena.
pub type ElementId = usize;

/// Bump-allocated arena for IR elements.
///
/// Elements are stored densely in a bump-backed vector and referenced by
/// index. This keeps element allocation fast and avoids per-node heap churn.
#[derive(Debug, Clone)]
pub struct IrArena<'a> {
    elements: IrVec<'a, Element<'a>>,
}

impl<'a> IrArena<'a> {
    /// Create a new empty arena in the provided bump allocator.
    pub fn new_in(arena: &'a Bump) -> Self {
        IrArena {
            elements: IrVec::new_in(arena),
        }
    }

    /// Create a new arena with the given capacity.
    pub fn with_capacity_in(capacity: usize, arena: &'a Bump) -> Self {
        IrArena {
            elements: IrVec::with_capacity_in(capacity, arena),
        }
    }

    /// Allocate a new element and return its ID.
    pub fn new_node(&mut self, element: Element<'a>) -> ElementId {
        let id = self.elements.len();
        self.elements.push(element);
        id
    }

    /// Reserve space for at least `additional` elements.
    pub fn reserve(&mut self, additional: usize) {
        self.elements.reserve(additional);
    }

    /// Returns the number of elements stored in the arena.
    pub fn count(&self) -> usize {
        self.elements.len()
    }

    /// Returns a reference to the element with the given ID.
    pub fn get(&self, id: ElementId) -> Option<&Element<'a>> {
        self.elements.get(id)
    }

    /// Returns a mutable reference to the element with the given ID.
    pub fn get_mut(&mut self, id: ElementId) -> Option<&mut Element<'a>> {
        self.elements.get_mut(id)
    }
}

/// Arena-backed IR tree.
///
/// The tree owns an `IrArena` of elements and stores the root node ID. All
/// element references inside `Node::Element` variants point back into this
/// arena.
#[derive(Debug, Clone)]
pub struct IrTree<'a> {
    arena: ManuallyDrop<IrArena<'a>>,
    root: ElementId,
}

impl<'a> IrTree<'a> {
    /// Create a new IR tree from the provided arena and root ID.
    pub fn new(arena: IrArena<'a>, root: ElementId) -> Self {
        IrTree {
            arena: ManuallyDrop::new(arena),
            root,
        }
    }

    /// Returns the root element ID.
    pub fn root(&self) -> ElementId {
        self.root
    }

    /// Returns a shared reference to the element arena.
    pub fn arena(&self) -> &IrArena<'a> {
        &self.arena
    }

    /// Returns a mutable reference to the element arena.
    pub fn arena_mut(&mut self) -> &mut IrArena<'a> {
        &mut self.arena
    }

    /// Returns the root element.
    pub fn root_element(&self) -> &Element<'a> {
        self.element(self.root)
    }

    /// Returns a reference to the element for the given ID.
    pub fn element(&self, id: ElementId) -> &Element<'a> {
        self.arena().get(id).expect("invalid element id")
    }

    /// Returns a mutable reference to the element for the given ID.
    pub fn element_mut(&mut self, id: ElementId) -> &mut Element<'a> {
        self.arena_mut().get_mut(id).expect("invalid element id")
    }
}

/// Returns true if the value should be considered "empty" for optional substitutions.
pub(crate) fn is_optional_empty(value: &BinXmlValue<'_>) -> bool {
    match value {
        BinXmlValue::NullType => true,
        BinXmlValue::StringType(s) => s.is_empty(),
        BinXmlValue::AnsiStringType(s) => s.is_empty(),
        BinXmlValue::BinaryType(bytes) => bytes.is_empty(),
        BinXmlValue::BinXmlType(bytes) => bytes.is_empty(),
        BinXmlValue::StringArrayType(v) => v.is_empty(),
        BinXmlValue::Int8ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt8ArrayType(v) => v.is_empty(),
        BinXmlValue::Int16ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt16ArrayType(v) => v.is_empty(),
        BinXmlValue::Int32ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt32ArrayType(v) => v.is_empty(),
        BinXmlValue::Int64ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt64ArrayType(v) => v.is_empty(),
        BinXmlValue::Real32ArrayType(v) => v.is_empty(),
        BinXmlValue::Real64ArrayType(v) => v.is_empty(),
        BinXmlValue::BoolArrayType(v) => v.is_empty(),
        BinXmlValue::GuidArrayType(v) => v.is_empty(),
        BinXmlValue::FileTimeArrayType(v) => v.is_empty(),
        BinXmlValue::SysTimeArrayType(v) => v.is_empty(),
        BinXmlValue::SidArrayType(v) => v.is_empty(),
        BinXmlValue::HexInt32ArrayType(v) => v.is_empty(),
        BinXmlValue::HexInt64ArrayType(v) => v.is_empty(),
        _ => false,
    }
}

/// Returns true if the template value should be considered "empty" for optional substitutions.
pub(crate) fn is_optional_empty_template_value(value: &TemplateValue<'_>) -> bool {
    match value {
        TemplateValue::BinXmlElement(_) => false,
        TemplateValue::Value(value) => is_optional_empty(value),
    }
}

#[cfg(test)]
mod drop_free_tests {
    use super::*;
    use crate::binxml::value_variant::BinXmlValue;

    #[test]
    fn ir_and_value_types_are_drop_free() {
        assert!(!std::mem::needs_drop::<Name<'static>>());
        assert!(!std::mem::needs_drop::<Text<'static>>());
        assert!(!std::mem::needs_drop::<BinXmlValue<'static>>());
        assert!(!std::mem::needs_drop::<Node<'static>>());
        assert!(!std::mem::needs_drop::<IrTree<'static>>());
    }
}
