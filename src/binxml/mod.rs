//! BinXML parsing and rendering internals.
//!
//! The crate exposes a small public `binxml` surface for consumers that need to
//! inspect decoded names or substitution values directly. Record parsing itself
//! primarily flows through two internal strategies:
//!
//! - `ir`: parse BinXML into the bump-allocated IR in [`crate::model::ir`] and
//!   render from that tree.
//! - `compiled_xml`: compile template definitions into static XML fragments and
//!   render substitution values directly from raw bytes on the hot XML path.

pub mod name;
pub mod value_variant;

pub(crate) mod array_expand;
pub(crate) mod compiled_xml;
pub(crate) mod ir;
pub(crate) mod ir_json;
pub(crate) mod ir_xml;
pub(crate) mod render_common;
pub(crate) mod tokens;
pub(crate) mod value_render;
pub(crate) mod xml_value_format;

pub use tokens::BinXmlTemplateValues;

/// Benchmark-only helpers for IR rendering.
#[cfg(feature = "bench")]
pub mod bench {
    use crate::EvtxChunk;
    use crate::err::Result;
    use crate::model::ir::{IrArena, Node};
    use bumpalo::Bump;
    use sonic_rs::writer::WriteExt;

    /// Invoke the internal JSON text rendering path on a pre-built node slice.
    pub fn write_json_text_content<'a, W: WriteExt>(
        writer: &mut W,
        arena: &'a IrArena<'a>,
        nodes: &[Node<'a>],
    ) -> Result<()> {
        super::ir_json::bench_write_json_text_content(writer, arena, nodes)
    }

    /// Benchmark-only wrapper around the template cache used in IR building.
    pub struct TreeBuildCache<'a> {
        cache: super::ir::IrTemplateCache<'a>,
    }

    impl<'a> TreeBuildCache<'a> {
        /// Create a cache tied to the chunk arena.
        pub fn new(chunk: &'a EvtxChunk<'a>) -> Self {
            TreeBuildCache {
                cache: super::ir::IrTemplateCache::new(&chunk.arena),
            }
        }
    }

    /// Build an IR tree from BinXML bytes using a caller-provided bump arena.
    pub fn build_tree_from_binxml_bytes_in_bump<'a>(
        bytes: &'a [u8],
        chunk: &'a EvtxChunk<'a>,
        cache: &mut TreeBuildCache<'a>,
        bump: &'a Bump,
    ) -> Result<usize> {
        // The production parser only has a direct (cursor-based) builder. Keep the benchmark API
        // stable by routing this helper to the direct implementation.
        super::ir::bench_build_tree_from_binxml_bytes_direct(bytes, chunk, &mut cache.cache, bump)
    }

    /// Build an IR tree directly from BinXML bytes without an iterator (bench-only).
    pub fn build_tree_from_binxml_bytes_direct_in_bump<'a>(
        bytes: &'a [u8],
        chunk: &'a EvtxChunk<'a>,
        cache: &mut TreeBuildCache<'a>,
        bump: &'a Bump,
    ) -> Result<usize> {
        super::ir::bench_build_tree_from_binxml_bytes_direct(bytes, chunk, &mut cache.cache, bump)
    }
}
