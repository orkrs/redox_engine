//! WGSL code generation for one material graph (MAT-02 step 3).
//!
//! The output is a function `material_surface_graph` plus the `MaterialParams` uniform
//! struct. The surrounding module (`SurfaceInput`, `SurfaceData`) comes from
//! `material_surface.wgsl`.

use crate::emit;
use crate::graph::Ty;
use crate::ir::{self, Ir};

/// Shader pass the variant is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    /// Forward / deferred main pass: every surface property.
    Main,
    /// Depth prepass: opacity only; other properties return zero.
    Depth,
    /// Shadow pass: opacity only; other properties return zero.
    Shadow,
}

/// Vertex and rasterisation features of a variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    /// Alpha-tested surface.
    pub alpha_test: bool,
    /// Skinned mesh.
    pub skinning: bool,
    /// Instanced draw.
    pub instanced: bool,
    /// Vertex animation (world position offset).
    pub vertex_anim: bool,
}

/// What to generate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodegenOptions {
    /// Pass the variant is for.
    pub pass: Pass,
    /// Features the variant is for.
    pub features: Features,
}

/// Generated WGSL text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    /// The `material_surface_graph` function.
    pub wgsl: String,
    /// The `MaterialParams` struct (one `vec4` per parameter slot, in slot order).
    pub uniform_wgsl: String,
    /// Size of the parameter block in bytes (16 per slot).
    pub param_bytes: usize,
}

/// Generates WGSL for `ir` under `opts`.
pub fn generate(ir: &Ir, opts: &CodegenOptions) -> Generated {
    let pruned = match opts.pass {
        Pass::Main => ir.clone(),
        Pass::Depth | Pass::Shadow => ir::opacity_only(ir),
    };
    let mut body = String::new();
    for op in &pruned.ops {
        body.push_str(&format!(
            "    let v{}: {} = {};\n",
            op.id,
            op.ty.wgsl(),
            emit::expr(&pruned, op)
        ));
    }
    let ret = return_expr(&pruned, opts.pass);
    let wgsl = format!(
        "// material graph: pass {:?}, features {:?}\n\
         fn material_surface_graph(in: SurfaceInput, uv: vec2<f32>, time: f32, params: MaterialParams) -> SurfaceData {{\n\
         {body}    return {ret};\n\
         }}\n",
        opts.pass, opts.features
    );
    Generated {
        wgsl,
        uniform_wgsl: uniform_struct(ir),
        param_bytes: ir.params.len() * 16,
    }
}

/// `SurfaceData(...)` constructor with the six graph values (zero for fields the pass drops).
fn return_expr(ir: &Ir, pass: Pass) -> String {
    let keep = |field: bool, id: u32, ty: Ty| {
        if field { format!("v{id}") } else { zero(ty) }
    };
    let main = pass == Pass::Main;
    let outs = ir.outputs;
    format!(
        "SurfaceData({}, {}, {}, {}, {}, {}, vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), 0.0)",
        keep(main, outs.base_color, Ty::Vec3),
        keep(true, outs.opacity, Ty::F32),
        keep(main, outs.normal, Ty::Vec3),
        keep(main, outs.metallic, Ty::F32),
        keep(main, outs.roughness, Ty::F32),
        keep(main, outs.emissive, Ty::Vec3),
    )
}

/// Zero value of a type, as WGSL.
fn zero(ty: Ty) -> String {
    match ty {
        Ty::F32 => "0.0".to_string(),
        Ty::Vec2 => "vec2<f32>(0.0)".to_string(),
        Ty::Vec3 => "vec3<f32>(0.0)".to_string(),
        Ty::Vec4 => "vec4<f32>(0.0)".to_string(),
        Ty::Texture2d | Ty::Sampler => "0.0".to_string(),
    }
}

/// `MaterialParams` struct: `p_<name>: vec4<f32>` per slot, in slot order. An empty uniform
/// gets one padding member because WGSL structs cannot be empty.
fn uniform_struct(ir: &Ir) -> String {
    let mut text = String::from("struct MaterialParams {\n");
    if ir.params.is_empty() {
        text.push_str("    _pad: vec4<f32>,\n");
    }
    for slot in &ir.params {
        text.push_str(&format!("    p_{}: vec4<f32>,\n", slot.name));
    }
    text.push_str("}\n");
    text
}
