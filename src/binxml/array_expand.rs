//! Array substitution expansion during template instantiation.
//!
//! MS-EVEN6 §3.1.4.7.5 ("Array Types") defines array values as a single substitution value in a
//! template instance that expands by **repeating the containing element** once per array item.
//!
//! This module implements that expansion on the IR tree produced by `clone_and_resolve`:
//! - We scan a resolved element for the first `Node::Value(<ArrayType>)` in its children/attrs.
//! - If found and the array has length > 1, we clone the element N times, replacing the array
//!   value with the Nth scalar item (or omitting it for empty strings).
//! - We recurse so multiple array substitutions in the same element expand deterministically.
//!
//! This keeps renderers dumb: XML/JSON just render the expanded structure, matching common tools
//! (notably `libevtx`) and the conceptual XML expansion in the spec.

use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{Attr, Element, ElementId, IrArena, IrVec, Node, Text};
use bumpalo::Bump;

pub(crate) fn node_needs_array_expansion(node: &Node<'_>) -> bool {
    let Node::Value(value) = node else {
        return false;
    };
    value.expandable_array_len().is_some_and(|len| len > 1)
}

/// Expand array substitutions inside a cloned element by repeating the element.
///
/// The input element (`element_id`) is expected to already be cloned into `arena`
/// (with placeholders resolved). If no expandable arrays are found, this returns
/// `None` (caller should keep `element_id` as-is).
pub(crate) fn expand_array_substitutions_in_element<'a>(
    arena: &mut IrArena<'a>,
    bump: &'a Bump,
    element_id: ElementId,
) -> Result<Option<Vec<ElementId>>> {
    // Expand one array occurrence at a time. If an element contains multiple arrays,
    // this produces a deterministic expansion (potentially a cross-product).
    let Some(expanded_once) = expand_first_array_in_element(arena, bump, element_id)? else {
        return Ok(None);
    };

    let mut out = Vec::with_capacity(expanded_once.len());
    for id in expanded_once {
        if let Some(expanded) = expand_array_substitutions_in_element(arena, bump, id)? {
            out.extend(expanded);
        } else {
            out.push(id);
        }
    }
    Ok(Some(out))
}

#[derive(Debug, Clone, Copy)]
enum ArrayLocation {
    Child(usize),
    Attr { attr_idx: usize, node_idx: usize },
}

#[derive(Debug, Clone)]
enum ScalarReplacement<'a> {
    /// Remove the substitution node entirely (producing an empty element / empty attribute value).
    Omit,
    /// Replace the substitution node with the provided scalar node.
    Node(Node<'a>),
}

fn expand_first_array_in_element<'a>(
    arena: &mut IrArena<'a>,
    bump: &'a Bump,
    element_id: ElementId,
) -> Result<Option<Vec<ElementId>>> {
    let (loc, array_value, len) = {
        let element = arena
            .get(element_id)
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("invalid element id"))?;

        let Some((loc, array_value, len)) = find_first_array_value(element) else {
            return Ok(None);
        };

        (loc, array_value, len)
    };

    if len <= 1 {
        return Ok(None);
    }

    let mut out = Vec::with_capacity(len);
    for idx in 0..len {
        let Some(replacement) = scalar_replacement_from_array_value(&array_value, idx) else {
            // Unknown/unsupported array representation: don't expand.
            return Ok(None);
        };

        let new_elem = {
            let element = arena
                .get(element_id)
                .ok_or_else(|| EvtxError::FailedToCreateRecordModel("invalid element id"))?;
            clone_element_with_replacement(bump, element, loc, &replacement)
        };

        out.push(arena.new_node(new_elem));
    }

    Ok(Some(out))
}

fn clone_element_with_replacement<'a>(
    bump: &'a Bump,
    element: &Element<'a>,
    loc: ArrayLocation,
    replacement: &ScalarReplacement<'a>,
) -> Element<'a> {
    let mut out = Element {
        name: element.name,
        attrs: IrVec::with_capacity_in(element.attrs.len(), bump),
        children: IrVec::with_capacity_in(element.children.len(), bump),
        has_element_child: element.has_element_child,
    };

    for (a_idx, attr) in element.attrs.iter().enumerate() {
        let mut new_attr = Attr {
            name: attr.name,
            value: IrVec::with_capacity_in(attr.value.len(), bump),
        };

        let replace_idx = match loc {
            ArrayLocation::Attr { attr_idx, node_idx } if attr_idx == a_idx => Some(node_idx),
            _ => None,
        };

        for (n_idx, node) in attr.value.iter().enumerate() {
            if replace_idx == Some(n_idx) {
                push_replacement(&mut new_attr.value, replacement);
            } else {
                new_attr.value.push(node.clone());
            }
        }

        if !new_attr.value.is_empty() {
            out.attrs.push(new_attr);
        }
    }

    let replace_child_idx = match loc {
        ArrayLocation::Child(pos) => Some(pos),
        _ => None,
    };

    for (c_idx, node) in element.children.iter().enumerate() {
        if replace_child_idx == Some(c_idx) {
            push_replacement(&mut out.children, replacement);
        } else {
            out.children.push(node.clone());
        }
    }

    out
}

fn push_replacement<'a>(out: &mut IrVec<'a, Node<'a>>, replacement: &ScalarReplacement<'a>) {
    match replacement {
        ScalarReplacement::Omit => {}
        ScalarReplacement::Node(node) => out.push(node.clone()),
    }
}

/// Find the first expandable array value inside `element`.
///
/// Scan order is deterministic (important for stable output):
/// 1. `element.children` (left-to-right)
/// 2. `element.attrs[*].value` (attribute order, then left-to-right)
///
/// Returns `(location, cloned_value, len)` so callers can drop borrows to `element`
/// before allocating/cloning repeated elements.
fn find_first_array_value<'a>(
    element: &Element<'a>,
) -> Option<(ArrayLocation, BinXmlValue<'a>, usize)> {
    for (idx, node) in element.children.iter().enumerate() {
        let Node::Value(value) = node else { continue };
        let Some(len) = value.expandable_array_len() else {
            continue;
        };
        if len <= 1 {
            // Single-item arrays don't require element repetition and should not block scanning
            // for later expandable arrays within the same element.
            continue;
        }
        return Some((ArrayLocation::Child(idx), value.clone(), len));
    }

    for (a_idx, attr) in element.attrs.iter().enumerate() {
        for (n_idx, node) in attr.value.iter().enumerate() {
            let Node::Value(value) = node else { continue };
            let Some(len) = value.expandable_array_len() else {
                continue;
            };
            if len <= 1 {
                continue;
            }
            return Some((
                ArrayLocation::Attr {
                    attr_idx: a_idx,
                    node_idx: n_idx,
                },
                value.clone(),
                len,
            ));
        }
    }

    None
}

/// Convert an array value into the scalar replacement for a single item at `idx`.
///
/// Empty strings map to `ScalarReplacement::Omit` to produce empty elements (e.g. `<Data/>`).
fn scalar_replacement_from_array_value<'a>(
    value: &BinXmlValue<'a>,
    idx: usize,
) -> Option<ScalarReplacement<'a>> {
    let scalar = value.array_item_as_value(idx)?;
    match scalar {
        // Strings are represented as `Node::Text` in the IR.
        BinXmlValue::StringType(s) => {
            if s.is_empty() {
                Some(ScalarReplacement::Omit)
            } else {
                Some(ScalarReplacement::Node(Node::Text(Text::utf16(s))))
            }
        }
        // Everything else stays as a typed value node.
        other => Some(ScalarReplacement::Node(Node::Value(other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParserSettings;
    use crate::binxml::ir_xml::render_xml_record;
    use crate::model::ir::IrTree;
    use crate::model::ir::Name;
    use crate::utils::Utf16LeSlice;

    /// Tiny helper to visualize small IR fragments as XML-ish strings in tests.
    ///
    /// This is intentionally incomplete (it only supports the node kinds we use in these tests),
    /// but it makes the array-expansion behavior much easier to grok at a glance.
    fn flat_xml<'a>(arena: &IrArena<'a>, id: ElementId) -> String {
        let e = arena.get(id).expect("element id must be valid");
        let mut out = String::new();

        out.push('<');
        out.push_str(e.name.as_str());
        out.push('>');

        for node in e.children.iter() {
            match node {
                Node::Text(Text::Utf8(s)) => out.push_str(s),
                Node::Text(Text::Utf16(s)) => out.push_str(&s.to_string().unwrap()),
                Node::Value(v) => out.push_str(&format!("{}", v)),
                other => panic!("unsupported node in flat_xml helper: {other:?}"),
            }
        }

        out.push_str("</");
        out.push_str(e.name.as_str());
        out.push('>');

        out
    }

    #[test]
    fn no_expansion_when_no_arrays_exist() {
        // Case: element has no array substitution nodes at all.
        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("EventData"), &bump));
        assert_eq!(
            expand_array_substitutions_in_element(&mut arena, &bump, root).unwrap(),
            None
        );
    }

    #[test]
    fn expands_string_array_child_into_repeated_elements() {
        // Case: array value appears as a child node (typical `<Data>%{0}</Data>` template).
        let a_bytes = [b'a', 0];
        let b_bytes = [b'b', 0];
        let a = Utf16LeSlice::new(&a_bytes, 1);
        let b = Utf16LeSlice::new(&b_bytes, 1);
        let items = [a, b];

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("Data"), &bump));

        arena
            .get_mut(root)
            .unwrap()
            .children
            .push(Node::Value(BinXmlValue::StringArrayType(&items)));

        let expanded = expand_array_substitutions_in_element(&mut arena, &bump, root)
            .unwrap()
            .expect("should expand");
        assert_eq!(expanded.len(), 2);

        let e0 = arena.get(expanded[0]).unwrap();
        assert_eq!(e0.name.as_str(), "Data");
        assert_eq!(e0.children.len(), 1);
        assert!(matches!(e0.children[0], Node::Text(Text::Utf16(s)) if s == a));

        let e1 = arena.get(expanded[1]).unwrap();
        assert_eq!(e1.name.as_str(), "Data");
        assert_eq!(e1.children.len(), 1);
        assert!(matches!(e1.children[0], Node::Text(Text::Utf16(s)) if s == b));
    }

    #[test]
    fn expands_numeric_array_in_attribute_value() {
        // Case: array value appears inside an attribute's node list.
        let items = [1i32, 2i32];

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("E"), &bump));
        let attr_name = Name::new("n");

        let mut attr = Attr {
            name: attr_name,
            value: IrVec::new_in(&bump),
        };
        attr.value
            .push(Node::Value(BinXmlValue::Int32ArrayType(&items)));
        arena.get_mut(root).unwrap().attrs.push(attr);

        let expanded = expand_array_substitutions_in_element(&mut arena, &bump, root)
            .unwrap()
            .expect("should expand");
        assert_eq!(expanded.len(), 2);

        let e0 = arena.get(expanded[0]).unwrap();
        assert_eq!(e0.attrs.len(), 1);
        assert_eq!(e0.attrs[0].name.as_str(), "n");
        assert_eq!(e0.attrs[0].value.len(), 1);
        assert!(matches!(
            e0.attrs[0].value[0],
            Node::Value(BinXmlValue::Int32Type(1))
        ));

        let e1 = arena.get(expanded[1]).unwrap();
        assert_eq!(e1.attrs.len(), 1);
        assert_eq!(e1.attrs[0].name.as_str(), "n");
        assert_eq!(e1.attrs[0].value.len(), 1);
        assert!(matches!(
            e1.attrs[0].value[0],
            Node::Value(BinXmlValue::Int32Type(2))
        ));
    }

    #[test]
    fn string_array_in_attribute_omits_attribute_when_item_is_empty() {
        // Case: string array is used in an attribute value. Empty string items should remove the
        // node, and if that makes the whole attribute empty, the attribute is omitted.
        let empty = Utf16LeSlice::empty();
        let x_bytes = [b'x', 0];
        let x = Utf16LeSlice::new(&x_bytes, 1);
        let items = [empty, x];

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("E"), &bump));
        let mut attr = Attr {
            name: Name::new("n"),
            value: IrVec::new_in(&bump),
        };
        attr.value
            .push(Node::Value(BinXmlValue::StringArrayType(&items)));
        arena.get_mut(root).unwrap().attrs.push(attr);

        let expanded = expand_array_substitutions_in_element(&mut arena, &bump, root)
            .unwrap()
            .expect("should expand");
        assert_eq!(expanded.len(), 2);

        let e0 = arena.get(expanded[0]).unwrap();
        assert!(e0.attrs.is_empty());

        let e1 = arena.get(expanded[1]).unwrap();
        assert_eq!(e1.attrs.len(), 1);
        assert_eq!(e1.attrs[0].name.as_str(), "n");
        assert_eq!(e1.attrs[0].value.len(), 1);
        assert!(matches!(e1.attrs[0].value[0], Node::Text(Text::Utf16(s)) if s == x));
    }

    #[test]
    fn preserves_surrounding_nodes_when_expanding_child_array() {
        // Case: array node is surrounded by other text/value nodes; only the array slot should be
        // replaced per expansion item.
        let items = [10i32, 20i32];

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("E"), &bump));
        {
            let e = arena.get_mut(root).unwrap();
            e.children.push(Node::Text(Text::utf8("pre")));
            e.children
                .push(Node::Value(BinXmlValue::Int32ArrayType(&items)));
            e.children.push(Node::Text(Text::utf8("post")));
        }

        let expanded = expand_array_substitutions_in_element(&mut arena, &bump, root)
            .unwrap()
            .expect("should expand");
        assert_eq!(expanded.len(), 2);

        let e0 = arena.get(expanded[0]).unwrap();
        assert_eq!(e0.children.len(), 3);
        assert!(matches!(e0.children[0], Node::Text(Text::Utf8("pre"))));
        assert!(matches!(
            e0.children[1],
            Node::Value(BinXmlValue::Int32Type(10))
        ));
        assert!(matches!(e0.children[2], Node::Text(Text::Utf8("post"))));

        let e1 = arena.get(expanded[1]).unwrap();
        assert_eq!(e1.children.len(), 3);
        assert!(matches!(e1.children[0], Node::Text(Text::Utf8("pre"))));
        assert!(matches!(
            e1.children[1],
            Node::Value(BinXmlValue::Int32Type(20))
        ));
        assert!(matches!(e1.children[2], Node::Text(Text::Utf8("post"))));
    }

    #[test]
    fn multiple_arrays_in_same_element_expand_deterministically() {
        // Case: multiple arrays in one element. We expand one array at a time (deterministic scan
        // order), which yields a Cartesian expansion.
        let a_items = [1u8, 2u8];
        let b_items = [10u8, 20u8, 30u8];

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("E"), &bump));
        {
            let e = arena.get_mut(root).unwrap();
            // First array appears first in children => expanded first.
            e.children
                .push(Node::Value(BinXmlValue::UInt8ArrayType(&a_items)));
            e.children
                .push(Node::Value(BinXmlValue::UInt8ArrayType(&b_items)));
        }

        let expanded = expand_array_substitutions_in_element(&mut arena, &bump, root)
            .unwrap()
            .expect("should expand");
        assert_eq!(expanded.len(), 6);

        // Expected order:
        // a=1 with b=10,20,30, then a=2 with b=10,20,30.
        let expected = [
            (1u8, 10u8),
            (1u8, 20u8),
            (1u8, 30u8),
            (2u8, 10u8),
            (2u8, 20u8),
            (2u8, 30u8),
        ];

        for (id, (a, b)) in expanded.iter().copied().zip(expected) {
            let e = arena.get(id).unwrap();
            assert_eq!(e.children.len(), 2);
            assert!(matches!(e.children[0], Node::Value(BinXmlValue::UInt8Type(v)) if v == a));
            assert!(matches!(e.children[1], Node::Value(BinXmlValue::UInt8Type(v)) if v == b));
        }
    }

    #[test]
    fn ignores_single_item_arrays_and_still_expands_later_arrays() {
        // Regression: if a single-item array appears before a multi-item array in the same
        // element, we must ignore the first and still expand the later one.
        //
        // Conceptual template fragment (after placeholder resolution, before array expansion):
        //
        //   <Data>%{0}:%{1}</Data>
        //
        // With substitution values:
        // - %{0} = UInt8ArrayType([7])            // len = 1 (does NOT trigger repetition)
        // - %{1} = UInt8ArrayType([10, 20, 30])   // len = 3 (DOES trigger repetition)
        //
        // Expected expanded tree (array substitution repeats the *containing element*):
        //
        //   <Data>7:10</Data>
        //   <Data>7:20</Data>
        //   <Data>7:30</Data>
        let first = [7u8];
        let second = [10u8, 20u8, 30u8];

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("Data"), &bump));
        {
            let e = arena.get_mut(root).unwrap();
            e.children
                .push(Node::Value(BinXmlValue::UInt8ArrayType(&first)));
            e.children.push(Node::Text(Text::utf8(":")));
            e.children
                .push(Node::Value(BinXmlValue::UInt8ArrayType(&second)));
        }

        let expanded = expand_array_substitutions_in_element(&mut arena, &bump, root)
            .unwrap()
            .expect("should expand later array");
        assert_eq!(expanded.len(), 3);

        for (id, expected_second) in expanded.iter().copied().zip([10u8, 20u8, 30u8]) {
            let e = arena.get(id).unwrap();
            assert_eq!(e.children.len(), 3);

            // The first array (len=1) is ignored (left as-is).
            assert!(matches!(
                e.children[0],
                Node::Value(BinXmlValue::UInt8ArrayType(v)) if v.len() == 1 && v[0] == 7
            ));

            assert!(matches!(e.children[1], Node::Text(Text::Utf8(":"))));

            // The second array is expanded into scalar values.
            assert!(matches!(
                e.children[2],
                Node::Value(BinXmlValue::UInt8Type(v)) if v == expected_second
            ));

            // Make it readable: show the "as if XML" output we expect.
            let got = flat_xml(&arena, id);
            assert_eq!(got, format!("<Data>7:{expected_second}</Data>"));
        }
    }

    #[test]
    fn spec_example_repeats_containing_element_for_each_array_item() {
        // MS-EVEN6 §3.1.4.7.5 ("Array Types") describes array substitution expansion as repeating
        // the *containing element* once per array item.
        //
        // Conceptual template:
        //   <SomeEvent>
        //     <PropA>%1</PropA>
        //     <PropB>%2</PropB>
        //   </SomeEvent>
        //
        // Values:
        //   %1 = UInt8Array([97, 99])
        //   %2 = UInt8(101)
        //
        // Expected XML:
        //   <SomeEvent><PropA>97</PropA><PropA>99</PropA><PropB>101</PropB></SomeEvent>
        let prop_a_items = [97u8, 99u8];
        let prop_b_value = 101u8;

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        // Build the element that directly contains the array substitution: <PropA>%1</PropA>.
        let prop_a = arena.new_node(Element::new_in(Name::new("PropA"), &bump));
        arena
            .get_mut(prop_a)
            .unwrap()
            .children
            .push(Node::Value(BinXmlValue::UInt8ArrayType(&prop_a_items)));

        // Expand array substitution in <PropA>.
        let expanded_prop_a = expand_array_substitutions_in_element(&mut arena, &bump, prop_a)
            .unwrap()
            .expect("should expand PropA");
        assert_eq!(expanded_prop_a.len(), 2);

        // Build the scalar element: <PropB>%2</PropB>.
        let prop_b = arena.new_node(Element::new_in(Name::new("PropB"), &bump));
        arena
            .get_mut(prop_b)
            .unwrap()
            .children
            .push(Node::Value(BinXmlValue::UInt8Type(prop_b_value)));

        // Build the parent element and splice the expanded PropA siblings into it.
        let some_event = arena.new_node(Element::new_in(Name::new("SomeEvent"), &bump));
        {
            let e = arena.get_mut(some_event).unwrap();
            for id in expanded_prop_a {
                e.push_child(Node::Element(id));
            }
            e.push_child(Node::Element(prop_b));
        }

        // Render the whole tree to XML to match the spec example.
        let tree = IrTree::new(arena, some_event);
        let settings = ParserSettings::default().indent(false);
        let mut out = Vec::new();
        render_xml_record(&tree, &settings, &mut out).unwrap();

        let xml = String::from_utf8(out).unwrap();
        assert_eq!(
            xml,
            concat!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
                "<SomeEvent><PropA>97</PropA><PropA>99</PropA><PropB>101</PropB></SomeEvent>"
            )
        );
    }

    #[test]
    fn empty_string_items_omit_the_node() {
        // Case: string array contains an empty item. We omit the node so the repeated element
        // becomes truly empty (`<Data/>`), matching typical EVTX output expectations.
        let empty = Utf16LeSlice::empty();
        let x_bytes = [b'x', 0];
        let x = Utf16LeSlice::new(&x_bytes, 1);
        let items = [empty, x];

        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let root = arena.new_node(Element::new_in(Name::new("Data"), &bump));

        arena
            .get_mut(root)
            .unwrap()
            .children
            .push(Node::Value(BinXmlValue::StringArrayType(&items)));

        let expanded = expand_array_substitutions_in_element(&mut arena, &bump, root)
            .unwrap()
            .expect("should expand");
        assert_eq!(expanded.len(), 2);

        let e0 = arena.get(expanded[0]).unwrap();
        assert!(e0.children.is_empty());

        let e1 = arena.get(expanded[1]).unwrap();
        assert_eq!(e1.children.len(), 1);
        assert!(matches!(e1.children[0], Node::Text(Text::Utf16(s)) if s == x));
    }
}
