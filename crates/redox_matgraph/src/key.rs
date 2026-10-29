//! Stable keys of shader variants (MAT-02 step 5, key part).
//!
//! The key hashes the generated WGSL, so it depends on the graph's content and on the pass
//! and features, but not on the graph name. FNV-1a keeps it stable across runs (the std
//! hasher is randomly seeded and must not be used for this).

use crate::codegen::{CodegenOptions, generate};
use crate::ir::Ir;

/// Identifier of one shader variant of a material graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderKey(pub u64);

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Key of the variant of `ir` built with `opts`.
pub fn variant_key(ir: &Ir, opts: &CodegenOptions) -> ShaderKey {
    let generated = generate(ir, opts);
    let hash = generated
        .uniform_wgsl
        .bytes()
        .chain(generated.wgsl.bytes())
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
        });
    ShaderKey(hash)
}
