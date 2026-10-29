//! Material graphs (MAT-02): a node graph is checked, lowered to SSA IR, optimised and
//! turned into WGSL for `material_surface_graph`. Pure Rust, no GPU dependency.
//!
//! Pipeline: [`MaterialGraph::from_ron`] -> [`MaterialGraph::validate`] -> [`lower`] ->
//! [`optimize`] -> [`generate`]. [`eval`] interprets the IR on the CPU and serves as the
//! reference for GPU comparisons.

pub mod check;
pub mod codegen;
mod emit;
pub mod eval;
pub mod graph;
pub mod ir;
pub mod key;

pub use check::node_types;
pub use codegen::{CodegenOptions, Features, Generated, Pass, generate};
pub use eval::{EvalInputs, SurfaceValues, Value, apply, eval};
pub use graph::{
    FORMAT_VERSION, GraphError, MaterialGraph, Node, NodeId, NodeKind, PortRef, Ty, arity,
    input_types, output_type,
};
pub use ir::{Ir, Op, OpKind, ParamSlot, SurfaceOutputs, lower, optimize, prune};
pub use key::{ShaderKey, variant_key};
