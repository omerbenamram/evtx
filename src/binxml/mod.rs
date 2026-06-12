pub mod name;
pub mod value_variant;

pub(crate) mod array_expand;
pub(crate) mod compiled;
pub(crate) mod ir;
pub(crate) mod tokens;
pub(crate) mod value_render;

pub use tokens::BinXmlTemplateValues;

/// Benchmark-only helpers for IR rendering.
#[doc(hidden)]
#[cfg(feature = "bench")]
pub mod bench {
    use crate::EvtxChunk;
    use crate::err::Result;
    use crate::model::ir::{IrArena, Node};
    use bumpalo::Bump;

    /// Invoke the internal JSON text rendering path on a pre-built node slice.
    pub fn write_json_text_content<'a>(
        writer: &mut Vec<u8>,
        arena: &'a IrArena<'a>,
        nodes: &[Node<'a>],
    ) -> Result<()> {
        let _ = arena;
        super::compiled::bench_json_text_content(writer, nodes)
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
        super::ir::bench_build_tree_from_binxml_bytes_direct(bytes, chunk, &mut cache.cache, bump)
    }
}
