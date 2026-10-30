//! Compiles one material graph into one shader variant (MAT-02 step 5).
//!
//! The pipeline is `lower` -> `optimize` -> `generate`; the key is the stable hash of the
//! generated text. Pure Rust, so the GPU-side cache can be tested against it without a device.

use crate::codegen::{CodegenOptions, Generated, generate};
use crate::graph::{GraphError, MaterialGraph};
use crate::ir::{Ir, lower, optimize};
use crate::key::{ShaderKey, variant_key};

/// One compiled variant of a graph for one pass and feature set.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    /// Key of the variant; equal graphs compiled with equal options give equal keys.
    pub key: ShaderKey,
    /// Generated WGSL and parameter layout.
    pub generated: Generated,
    /// Optimised IR the WGSL was generated from (kept for the CPU reference).
    pub ir: Ir,
}

/// Checks, lowers, optimises and generates `graph` for `opts`.
pub fn compile_variant(
    graph: &MaterialGraph,
    opts: &CodegenOptions,
) -> Result<Variant, Vec<GraphError>> {
    let ir = optimize(lower(graph)?);
    let key = variant_key(&ir, opts);
    let generated = generate(&ir, opts);
    Ok(Variant { key, generated, ir })
}
