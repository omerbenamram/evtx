pub mod deserializer;
pub mod name;
pub mod value_variant;

pub(crate) mod array_expand;
pub(crate) mod ir;
pub(crate) mod ir_json;
pub(crate) mod ir_xml;
pub(crate) mod tokens;
pub(crate) mod value_render;

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
        super::ir::bench_build_tree_from_binxml_bytes(bytes, chunk, &mut cache.cache, bump)
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
