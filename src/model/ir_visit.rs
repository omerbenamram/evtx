//! Visitor utilities for the IR element tree.
//!
//! This module defines a lightweight visitor trait and a depth-first traversal
//! helper for walking the IR (`model::ir`). The visitor operates on resolved
//! trees and is intended for rendering or analysis without rebuilding token
//! streams or allocating additional intermediate structures.
//!
//! Traversal behavior:
//! - `walk_ir` calls `start_element` before visiting children and `end_element`
//!   after all children are processed.
//! - Non-element nodes are visited in document order as they appear in the
//!   `Element::children` list.
//! - Attribute handling is left to the visitor; attributes are accessible via
//!   the `Element` passed to `start_element`.
//! - Placeholder nodes should not appear in resolved trees. Visitors may treat
//!   them as errors if encountered.

use crate::binxml::value_variant::BinXmlValue;
use crate::model::ir::{Element, ElementId, IrTree, Name, Node, Placeholder, Text};

/// Visitor interface for traversing an IR element tree.
pub trait IrVisitor {
    /// Error type returned by visitor callbacks.
    type Error;

    /// Called when entering an element node (pre-order).
    fn start_element(&mut self, element: &Element<'_>) -> Result<(), Self::Error>;

    /// Called when leaving an element node (post-order).
    fn end_element(&mut self, element: &Element<'_>) -> Result<(), Self::Error>;

    /// Called for plain text nodes.
    fn visit_text(&mut self, text: &Text<'_>) -> Result<(), Self::Error>;

    /// Called for value nodes.
    fn visit_value(&mut self, value: &BinXmlValue<'_>) -> Result<(), Self::Error>;

    /// Called for entity references.
    fn visit_entity_ref(&mut self, name: &Name<'_>) -> Result<(), Self::Error>;

    /// Called for character references.
    fn visit_char_ref(&mut self, value: u16) -> Result<(), Self::Error>;

    /// Called for CDATA nodes.
    fn visit_cdata(&mut self, text: &Text<'_>) -> Result<(), Self::Error>;

    /// Called for processing instruction targets.
    fn visit_pi_target(&mut self, name: &Name<'_>) -> Result<(), Self::Error>;

    /// Called for processing instruction data.
    fn visit_pi_data(&mut self, text: &Text<'_>) -> Result<(), Self::Error>;

    /// Called for placeholder nodes.
    fn visit_placeholder(&mut self, placeholder: &Placeholder) -> Result<(), Self::Error>;
}

/// Depth-first walk of an IR element tree.
pub fn walk_ir<V: IrVisitor>(tree: &IrTree<'_>, visitor: &mut V) -> Result<(), V::Error> {
    walk_ir_node(tree, tree.root(), visitor)
}

fn walk_ir_node<V: IrVisitor>(
    tree: &IrTree<'_>,
    element_id: ElementId,
    visitor: &mut V,
) -> Result<(), V::Error> {
    let element = tree.element(element_id);
    visitor.start_element(element)?;

    for node in &element.children {
        match node {
            Node::Element(child_id) => walk_ir_node(tree, *child_id, visitor)?,
            Node::Text(text) => visitor.visit_text(text)?,
            Node::Value(value) => visitor.visit_value(value)?,
            Node::EntityRef(name) => visitor.visit_entity_ref(name)?,
            Node::CharRef(value) => visitor.visit_char_ref(*value)?,
            Node::CData(text) => visitor.visit_cdata(text)?,
            Node::PITarget(name) => visitor.visit_pi_target(name)?,
            Node::PIData(text) => visitor.visit_pi_data(text)?,
            Node::Placeholder(ph) => visitor.visit_placeholder(ph)?,
        }
    }

    visitor.end_element(element)?;
    Ok(())
}
