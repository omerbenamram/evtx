pub mod deserializer;
pub mod name;
pub mod value_variant;

pub(crate) mod ir;
pub(crate) mod ir_json;
pub(crate) mod ir_xml;
pub(crate) mod value_render;
pub(crate) mod tokens;

/// Benchmark-only helpers for IR rendering.
#[cfg(feature = "bench")]
pub mod bench {
    use bumpalo::Bump;
    use crate::err::Result;
    use crate::model::ir::{IrArena, Node};
    use crate::EvtxChunk;
    use sonic_rs::writer::WriteExt;

    #[cfg(feature = "perf-counters")]
    const JSON_WRITE_BUCKETS: usize = 9;

    /// Invoke the internal JSON text rendering path on a pre-built node slice.
    pub fn write_json_text_content<'a, W: WriteExt>(
        writer: &mut W,
        arena: &'a IrArena<'a>,
        nodes: &[Node<'a>],
    ) -> Result<()> {
        super::ir_json::bench_write_json_text_content(writer, arena, nodes)
    }

    /// Snapshot of JSON writer call/size statistics (perf-counters only).
    #[cfg(feature = "perf-counters")]
    #[derive(Debug, Clone, Copy)]
    pub struct JsonWriterStats {
        pub calls: u64,
        pub bytes: u64,
        pub max_write: u64,
        pub buckets: [u64; JSON_WRITE_BUCKETS],
    }

    /// Reset JSON writer counters (perf-counters only).
    #[cfg(feature = "perf-counters")]
    pub fn reset_json_writer_stats() {
        super::ir_json::perf::reset();
    }

    /// Read JSON writer counters (perf-counters only).
    #[cfg(feature = "perf-counters")]
    pub fn json_writer_stats() -> JsonWriterStats {
        let stats = super::ir_json::perf::snapshot();
        JsonWriterStats {
            calls: stats.calls,
            bytes: stats.bytes,
            max_write: stats.max_write,
            buckets: stats.buckets,
        }
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
