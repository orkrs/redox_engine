//! Material graphs (MAT-02, extended by MAT-03): a node graph is checked, lowered to SSA IR,
//! optimised and turned into WGSL for `material_surface_graph`. Pure Rust, no GPU dependency.
//!
//! Pipeline: [`MaterialGraph::from_ron`] -> [`MaterialGraph::validate`] -> [`lower`] ->
//! [`optimize`] -> [`generate`]. [`fn@eval`] interprets the IR on the CPU and serves as the
//! reference for GPU comparisons ([`try_eval`] refuses graphs with custom WGSL nodes).
//!
//! Generated code expects these bindings from the renderer: `tex<slot>: texture_2d<f32>` for
//! each texture slot, and the samplers `samp_default`, `samp_repeat`, `samp_clamp`.

pub mod check;
pub mod codegen;
pub mod custom;
mod emit;
pub mod eval;
pub mod graph;
pub mod ir;
pub mod key;
pub mod noise;
pub mod texture;
pub mod variant;
pub mod watch;

pub use check::{GraphWarning, node_types};
pub use codegen::{CodegenOptions, Features, Generated, Pass, generate};
pub use eval::{
    EvalEnv, EvalInputs, FlatTexture, SurfaceValues, TextureSampler, Value, apply, eval,
    eval_with, try_eval,
};
pub use graph::{
    FORMAT_VERSION, GraphError, MaterialGraph, Node, NodeId, NodeKind, PortRef, Ty, arity,
    input_types, output_type,
};
pub use ir::{
    Ir, MathFn, Op, OpKind, ParamSlot, SamplerKind, SurfaceOutputs, lower, optimize, prune,
};
pub use key::{ShaderKey, variant_key};
pub use noise::NoiseKind;
pub use variant::{Variant, compile_variant};
pub use watch::{GraphReload, MatGraphWatcher, load_graph_file};
