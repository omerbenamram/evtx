//! Render-time template resolution.
//!
//! Records whose BinXML stream is a single `TemplateInstance` are rendered
//! directly from the cached template definition tree instead of deep-cloning
//! it per record (see `RecordContent::Template`). The renderers walk the
//! cached tree and resolve `Node::Placeholder`s against the instance's
//! substitution values on the fly via [`Scope::resolve`].
//!
//! The resolution rules mirror `binxml::ir::resolve_placeholder_into` /
//! `value_to_node`, and render-time array expansion mirrors
//! `binxml::array_expand` (containing-element repetition, cross-product for
//! multiple arrays, empty string items omit the node). The materialized path
//! remains the source of truth for fallback record shapes; any behavior
//! change here is an output regression.

use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{
    Element, ElementId, IrArena, Node, Placeholder, TemplateValue, Text, is_optional_empty,
    is_optional_empty_template_value,
};

/// Per-record template resolution context: the substitution values, the arena
/// holding pre-parsed embedded BinXML fragments, and the record's nested
/// (unmaterialized) template instances.
#[derive(Clone, Copy)]
pub(crate) struct TplCtx<'t, 'a> {
    pub values: &'t [TemplateValue<'a>],
    pub frags: &'t IrArena<'a>,
    pub nested: &'t [crate::binxml::ir::TplInstance<'a>],
    /// False when neither the template nor this record's values contain any
    /// expandable array — lets expansion scans collapse to one branch.
    pub may_expand: bool,
}

/// The arena an element lives in, plus the optional template-resolution context.
///
/// `ctx: None` means the tree is fully materialized (record arena or fragment
/// arena) and every node resolves to itself.
#[derive(Clone, Copy)]
pub(crate) struct Scope<'t, 'a> {
    pub arena: &'t IrArena<'a>,
    pub ctx: Option<TplCtx<'t, 'a>>,
}

/// One array-expansion override frame: while rendering copy `idx` of an
/// expanded element, the array node identified by `slot` (its address in the
/// cached template tree) resolves to item `idx`. Frames chain through
/// `parent` for cross-product expansion of multiple arrays.
#[derive(Clone, Copy)]
pub(crate) struct Ovr<'p> {
    slot: usize,
    idx: usize,
    parent: Option<&'p Ovr<'p>>,
}

fn ovr_lookup(mut ovr: Option<&Ovr<'_>>, node: &Node<'_>) -> Option<usize> {
    let addr = node as *const Node<'_> as usize;
    while let Some(o) = ovr {
        if o.slot == addr {
            return Some(o.idx);
        }
        ovr = o.parent;
    }
    None
}

/// A template node as seen by the renderers after resolution.
pub(crate) enum RNode<'n, 'a> {
    /// A regular node (no placeholder involved).
    Plain(&'n Node<'a>),
    /// A resolved string substitution (mirrors `value_to_node`).
    Text(Text<'a>),
    /// A resolved scalar/array substitution.
    Value(&'n BinXmlValue<'a>),
    /// An array-expansion item (owned scalar).
    OwnValue(BinXmlValue<'a>),
    /// A resolved embedded-BinXML substitution: element in `TplCtx::frags`.
    Frag(ElementId),
    /// A resolved nested template instance: index into `TplCtx::nested`.
    Nested(u16),
    /// An omitted substitution (optional-empty, out-of-range, or empty string item).
    Skip,
}

/// Array item -> render node, mirroring `array_expand::scalar_replacement_from_array_value`.
fn item_rnode<'n, 'a>(value: &BinXmlValue<'a>, idx: usize) -> RNode<'n, 'a> {
    match value.array_item_as_value(idx) {
        Some(BinXmlValue::StringType(s)) => {
            if s.is_empty() {
                RNode::Skip
            } else {
                RNode::Text(Text::utf16(s))
            }
        }
        Some(other) => RNode::OwnValue(other),
        // Unreachable for expandable array types; keep fail-soft.
        None => RNode::Skip,
    }
}

impl<'t, 'a> Scope<'t, 'a> {
    pub(crate) fn materialized(arena: &'t IrArena<'a>) -> Self {
        Scope { arena, ctx: None }
    }

    /// Scope for rendering an embedded fragment element (fully materialized).
    pub(crate) fn frag_scope(&self) -> Scope<'t, 'a> {
        let ctx = self.ctx.as_ref().expect("frag scope without template ctx");
        Scope {
            arena: ctx.frags,
            ctx: None,
        }
    }

    /// Scope + root element id for rendering a nested template instance.
    ///
    /// The nested root is never array-expanded (matches the materialized path,
    /// which discards the instantiation root's expansion flag).
    pub(crate) fn nested_scope_root(&self, idx: u16) -> (Scope<'t, 'a>, ElementId) {
        let ctx = self
            .ctx
            .as_ref()
            .expect("nested scope without template ctx");
        let inst = &ctx.nested[idx as usize];
        let scope = Scope {
            arena: inst.template.arena(),
            ctx: Some(TplCtx {
                values: inst.values.as_slice(),
                frags: ctx.frags,
                nested: ctx.nested,
                may_expand: inst.may_expand,
            }),
        };
        (scope, inst.template.root())
    }

    #[inline(always)]
    pub(crate) fn resolve<'n>(
        &self,
        node: &'n Node<'a>,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<RNode<'n, 'a>>
    where
        't: 'n,
    {
        let Some(ctx) = &self.ctx else {
            return Ok(RNode::Plain(node));
        };
        match node {
            Node::Placeholder(ph) => resolve_placeholder(ctx, ph, node, ovr),
            Node::Value(v) if ovr.is_some() => match ovr_lookup(ovr, node) {
                Some(idx) => Ok(item_rnode(v, idx)),
                None => Ok(RNode::Plain(node)),
            },
            _ => Ok(RNode::Plain(node)),
        }
    }
}

/// Mirrors `resolve_placeholder_into` + `value_to_node`.
#[inline]
fn resolve_placeholder<'n, 'a>(
    ctx: &TplCtx<'n, 'a>,
    ph: &Placeholder,
    node: &'n Node<'a>,
    ovr: Option<&Ovr<'_>>,
) -> Result<RNode<'n, 'a>> {
    let Some(tv) = ctx.values.get(ph.id as usize) else {
        return Ok(RNode::Skip);
    };
    if ovr.is_some()
        && let Some(idx) = ovr_lookup(ovr, node)
    {
        return match tv {
            TemplateValue::Value(v) => Ok(item_rnode(v, idx)),
            TemplateValue::BinXmlElement(_) | TemplateValue::NestedTemplate(_) => Ok(RNode::Skip),
        };
    }
    if ph.optional && is_optional_empty_template_value(tv) {
        return Ok(RNode::Skip);
    }
    match tv {
        TemplateValue::BinXmlElement(id) => Ok(RNode::Frag(*id)),
        TemplateValue::NestedTemplate(idx) => Ok(RNode::Nested(*idx)),
        TemplateValue::Value(v) => match v {
            BinXmlValue::EvtXml => Err(EvtxError::FailedToCreateRecordModel(
                "Unimplemented - EvtXml",
            )),
            BinXmlValue::BinXmlType(_) => Err(EvtxError::FailedToCreateRecordModel(
                "unsupported BinXML value in template substitution",
            )),
            BinXmlValue::EvtHandle => Err(EvtxError::FailedToCreateRecordModel(
                "unsupported BinXML value in tree",
            )),
            BinXmlValue::StringType(s) => Ok(RNode::Text(Text::utf16(*s))),
            BinXmlValue::AnsiStringType(s) => Ok(RNode::Text(Text::utf8(s))),
            other => Ok(RNode::Value(other)),
        },
    }
}

fn expandable_len(node: &Node<'_>, ctx: &TplCtx<'_, '_>) -> Option<usize> {
    match node {
        Node::Value(v) => v.expandable_array_len(),
        Node::Placeholder(ph) => {
            let tv = ctx.values.get(ph.id as usize)?;
            if ph.optional && is_optional_empty_template_value(tv) {
                return None;
            }
            match tv {
                TemplateValue::Value(v) => v.expandable_array_len(),
                TemplateValue::BinXmlElement(_) | TemplateValue::NestedTemplate(_) => None,
            }
        }
        _ => None,
    }
}

/// Find the first not-yet-overridden expandable array (len > 1) in `element`,
/// mirroring `array_expand::find_first_array_value` scan order:
/// children left-to-right, then attribute values in order.
///
/// Returns `(slot_address, len)`.
pub(crate) fn find_expansion(
    element: &Element<'_>,
    ctx: &TplCtx<'_, '_>,
    ovr: Option<&Ovr<'_>>,
) -> Option<(usize, usize)> {
    if !ctx.may_expand {
        return None;
    }
    let scan = |node: &Node<'_>| -> Option<(usize, usize)> {
        let len = expandable_len(node, ctx)?;
        if len <= 1 || ovr_lookup(ovr, node).is_some() {
            return None;
        }
        Some((node as *const Node<'_> as usize, len))
    };
    for node in &element.children {
        if let Some(hit) = scan(node) {
            return Some(hit);
        }
    }
    for attr in &element.attrs {
        for node in &attr.value {
            if let Some(hit) = scan(node) {
                return Some(hit);
            }
        }
    }
    None
}

impl<'p> Ovr<'p> {
    pub(crate) fn frame(slot: usize, idx: usize, parent: Option<&'p Ovr<'p>>) -> Ovr<'p> {
        Ovr { slot, idx, parent }
    }
}

/// Invoke `f` once per array-expansion copy of `element` (once total when no
/// expansion applies). Copies follow `array_expand` cross-product order.
pub(crate) fn for_each_expansion<'a, F>(
    scope: &Scope<'_, 'a>,
    element: &Element<'a>,
    ovr: Option<&Ovr<'_>>,
    f: &mut F,
) -> Result<()>
where
    F: FnMut(Option<&Ovr<'_>>) -> Result<()>,
{
    if let Some(ctx) = &scope.ctx
        && let Some((slot, len)) = find_expansion(element, ctx, ovr)
    {
        for idx in 0..len {
            let frame = Ovr::frame(slot, idx, ovr);
            for_each_expansion(scope, element, Some(&frame), f)?;
        }
        return Ok(());
    }
    f(ovr)
}

/// Early-exit variant of [`for_each_expansion`]: true if `f` is true for any copy.
pub(crate) fn expansion_any<'a, F>(
    scope: &Scope<'_, 'a>,
    element: &Element<'a>,
    ovr: Option<&Ovr<'_>>,
    f: &mut F,
) -> Result<bool>
where
    F: FnMut(Option<&Ovr<'_>>) -> Result<bool>,
{
    if let Some(ctx) = &scope.ctx
        && let Some((slot, len)) = find_expansion(element, ctx, ovr)
    {
        for idx in 0..len {
            let frame = Ovr::frame(slot, idx, ovr);
            if expansion_any(scope, element, Some(&frame), f)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    f(ovr)
}

/// Number of array-expansion copies `element` renders as (1 when none apply).
pub(crate) fn count_expansion_copies(
    scope: &Scope<'_, '_>,
    element: &Element<'_>,
    ovr: Option<&Ovr<'_>>,
) -> usize {
    if let Some(ctx) = &scope.ctx
        && let Some((slot, len)) = find_expansion(element, ctx, ovr)
    {
        // Later arrays are independent of the item index, so probe with idx 0.
        let frame = Ovr::frame(slot, 0, ovr);
        return len * count_expansion_copies(scope, element, Some(&frame));
    }
    1
}

/// A child node resolved to an element, with the scope it renders under.
pub(crate) struct ChildElement<'t, 'a> {
    pub scope: Scope<'t, 'a>,
    pub element: &'t Element<'a>,
    /// Whether render-time array expansion applies. False for nested template
    /// instance roots (instantiation roots are never repeated).
    pub expand: bool,
}

/// Resolve a child node to an element (plain child, spliced fragment, or
/// nested template instance root).
#[inline]
pub(crate) fn resolve_child_element<'t, 'a>(
    scope: Scope<'t, 'a>,
    node: &'t Node<'a>,
    ovr: Option<&Ovr<'_>>,
) -> Result<Option<ChildElement<'t, 'a>>> {
    match scope.resolve(node, ovr)? {
        RNode::Plain(Node::Element(child_id)) => {
            let element = scope.arena.get(*child_id).expect("invalid element id");
            Ok(Some(ChildElement {
                scope,
                element,
                expand: true,
            }))
        }
        RNode::Frag(child_id) => {
            let frag_scope = scope.frag_scope();
            let element = frag_scope.arena.get(child_id).expect("invalid element id");
            Ok(Some(ChildElement {
                scope: frag_scope,
                element,
                expand: true,
            }))
        }
        RNode::Nested(idx) => {
            let (nested_scope, root) = scope.nested_scope_root(idx);
            let element = nested_scope.arena.get(root).expect("invalid element id");
            Ok(Some(ChildElement {
                scope: nested_scope,
                element,
                expand: false,
            }))
        }
        _ => Ok(None),
    }
}

/// Infallible per-node classification for scan loops.
///
/// Substitutions whose resolution would error (EvtXml etc.) classify as
/// `Content`; the error still surfaces when the emitting pass resolves the
/// same node with [`Scope::resolve`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanClass {
    /// Resolves to nothing (omitted substitution / empty-string array item).
    Skip,
    /// An element (plain child, fragment, or nested instance).
    Element,
    /// Non-empty text-like content (text, value, char/entity ref).
    Content,
    /// Present but contributes no content (empty text/value, PI).
    Empty,
}

fn item_class(value: &BinXmlValue<'_>, idx: usize) -> ScanClass {
    match value.array_item_as_value(idx) {
        Some(BinXmlValue::StringType(s)) => {
            if s.is_empty() {
                ScanClass::Skip
            } else {
                ScanClass::Content
            }
        }
        Some(_) => ScanClass::Content,
        None => ScanClass::Skip,
    }
}

/// Classify one node for scanning purposes (see [`ScanClass`]).
#[inline(always)]
pub(crate) fn scan_class(
    scope: &Scope<'_, '_>,
    node: &Node<'_>,
    ovr: Option<&Ovr<'_>>,
) -> ScanClass {
    match node {
        Node::Element(_) => ScanClass::Element,
        Node::Text(text) | Node::CData(text) => {
            if text.is_empty() {
                ScanClass::Empty
            } else {
                ScanClass::Content
            }
        }
        Node::Value(value) => {
            if ovr.is_some()
                && scope.ctx.is_some()
                && let Some(idx) = ovr_lookup(ovr, node)
            {
                return item_class(value, idx);
            }
            if is_optional_empty(value) {
                ScanClass::Empty
            } else {
                ScanClass::Content
            }
        }
        Node::CharRef(_) | Node::EntityRef(_) => ScanClass::Content,
        Node::PITarget(_) | Node::PIData(_) => ScanClass::Empty,
        Node::Placeholder(ph) => {
            let Some(ctx) = &scope.ctx else {
                // Materialized trees should not contain placeholders; treat as
                // content so the emitting pass reports the error.
                return ScanClass::Content;
            };
            let Some(tv) = ctx.values.get(ph.id as usize) else {
                return ScanClass::Skip;
            };
            if ovr.is_some()
                && let Some(idx) = ovr_lookup(ovr, node)
            {
                return match tv {
                    TemplateValue::Value(v) => item_class(v, idx),
                    _ => ScanClass::Skip,
                };
            }
            if ph.optional && is_optional_empty_template_value(tv) {
                return ScanClass::Skip;
            }
            match tv {
                TemplateValue::BinXmlElement(_) | TemplateValue::NestedTemplate(_) => {
                    ScanClass::Element
                }
                TemplateValue::Value(v) => {
                    if is_optional_empty(v) {
                        ScanClass::Empty
                    } else {
                        ScanClass::Content
                    }
                }
            }
        }
    }
}

/// Returns true if `nodes` contains any semantically non-empty "text-like"
/// content (resolved against the scope).
#[inline]
pub(crate) fn has_non_empty_text_content(
    scope: &Scope<'_, '_>,
    nodes: &[Node<'_>],
    ovr: Option<&Ovr<'_>>,
) -> bool {
    nodes
        .iter()
        .any(|node| scan_class(scope, node, ovr) == ScanClass::Content)
}

/// Content facts for an element's children, computed in one scan pass:
/// `(has_text, has_element_child)` — "has any non-empty text-like content"
/// and "has any element-like child".
pub(crate) fn content_layout(
    scope: &Scope<'_, '_>,
    element: &Element<'_>,
    ovr: Option<&Ovr<'_>>,
) -> (bool, bool) {
    let mut has_text = false;
    let mut has_element_child = element.has_element_child;
    for node in &element.children {
        match scan_class(scope, node, ovr) {
            ScanClass::Content => has_text = true,
            ScanClass::Element => has_element_child = true,
            _ => {}
        }
        if has_text && has_element_child {
            break;
        }
    }
    (has_text, has_element_child)
}

/// Layout facts for an element's children, computed in one scan pass:
/// `(logically_empty, has_element_child)`.
///
/// "Logically empty" matches a materialized element with an empty `children`
/// vec (every node resolves to nothing); `has_element_child` accounts for
/// placeholders that resolve to embedded fragment elements.
pub(crate) fn child_layout(
    scope: &Scope<'_, '_>,
    element: &Element<'_>,
    ovr: Option<&Ovr<'_>>,
) -> (bool, bool) {
    if element.children.is_empty() {
        return (true, false);
    }
    if scope.ctx.is_none() {
        return (false, element.has_element_child);
    }
    let mut any = false;
    let mut has_element_child = element.has_element_child;
    for node in &element.children {
        match scan_class(scope, node, ovr) {
            ScanClass::Skip => {}
            ScanClass::Element => {
                any = true;
                has_element_child = true;
            }
            _ => {
                any = true;
            }
        }
        if any && has_element_child {
            break;
        }
    }
    (!any, has_element_child)
}
