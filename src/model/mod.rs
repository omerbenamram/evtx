//! Shared intermediate-representation modules used after BinXML parsing.
//!
//! [`ir`] contains the arena-backed tree used to represent a decoded EVTX record,
//! while [`ir_visit`] provides a small visitor API for walking that tree without
//! binding callers to the concrete renderer implementations.

/// Arena-backed IR node and tree types.
pub mod ir;
/// Visitor helpers for traversing IR trees.
pub mod ir_visit;
