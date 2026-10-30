//! WGSL code generation for one material graph (MAT-02 step 3).
//!
//! The output is a function `material_surface_graph` plus the `MaterialParams` uniform
//! struct. The surrounding module (`SurfaceInput`, `SurfaceData`) comes from
//! `material_surface.wgsl`.

use crate::custom;
use crate::emit;
use crate::graph::Ty;
use crate::ir::{self, Ir, OpKind};
use crate::noise;
use crate::texture;

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
    let offset_ir = ir.offset.map(|id| {
        ir::prune(Ir {
            ops: ir.ops.clone(),
            params: ir.params.clone(),
            outputs: ir::SurfaceOutputs {
                base_color: id,
                opacity: id,
                normal: id,
                metallic: id,
                roughness: id,
                emissive: id,
            },
            offset: None,
            wpo_max: 0.0,
        })
    });
    let all_irs: Vec<&Ir> = std::iter::once(&pruned).chain(offset_ir.as_ref()).collect();
    let helpers = helper_text(&all_irs);
    let customs = custom_text(&all_irs);
    let vertex_fn = vertex_offset_text(offset_ir.as_ref());
    let wgsl = format!(
        "// material graph: pass {:?}, features {:?}\n\
         {helpers}{customs}\
         fn material_surface_graph(in: SurfaceInput, uv: vec2<f32>, time: f32, params: MaterialParams) -> SurfaceData {{\n\
         {body}    return {ret};\n\
         }}\n{vertex_fn}",
        opts.pass, opts.features
    );
    Generated {
        wgsl,
        uniform_wgsl: uniform_struct(ir),
        param_bytes: ir.params.len() * 16,
    }
}

/// Helper functions the ops of `irs` call: noise, texture and bump helpers, emitted once when needed.
fn helper_text(irs: &[&Ir]) -> String {
    let any =
        |f: &dyn Fn(&OpKind) -> bool| irs.iter().any(|ir| ir.ops.iter().any(|op| f(&op.kind)));
    let mut text = String::new();
    if any(&|k| matches!(k, OpKind::Noise(_))) {
        text.push_str(noise::WGSL);
    }
    if any(&|k| {
        matches!(
            k,
            OpKind::Texture { .. } | OpKind::NormalMap { .. } | OpKind::Triplanar { .. }
        )
    }) {
        text.push_str(texture::WGSL);
    }
    if any(&|k| matches!(k, OpKind::NormalFromHeight(_))) {
        text.push_str(BUMP_WGSL);
    }
    text
}

/// Bump mapping from screen-space derivatives (Mikkelsen): perturbs `n` by the gradient of `h`.
const BUMP_WGSL: &str =
    "fn mg_bump_normal(h: f32, strength: f32, pos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let sigma_x = dpdx(pos);
    let sigma_y = dpdy(pos);
    let r1 = cross(sigma_y, n);
    let r2 = cross(n, sigma_x);
    let det = dot(sigma_x, r1);
    let grad = sign(det) * (dpdx(h) * r1 + dpdy(h) * r2);
    return normalize(abs(det) * n - strength * grad);
}
";

/// One WGSL function per custom node of `irs` (a node used by two of them is emitted once).
fn custom_text(irs: &[&Ir]) -> String {
    let mut seen = std::collections::HashSet::new();
    irs.iter()
        .flat_map(|ir| ir.ops.iter())
        .filter_map(|op| match &op.kind {
            OpKind::Custom {
                node,
                inputs,
                output,
                body,
            } if seen.insert(*node) => Some(custom::function_text(*node, inputs, *output, body)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `material_vertex_offset_graph`: the vertex offset of the graph (zero without a
/// `PositionOffset` node), with the same inputs as the surface function.
fn vertex_offset_text(offset: Option<&Ir>) -> String {
    let (body, ret) = match offset {
        None => (String::new(), "vec3<f32>(0.0)".to_string()),
        Some(sub) => {
            let mut body = String::new();
            for op in &sub.ops {
                body.push_str(&format!(
                    "    let v{}: {} = {};\n",
                    op.id,
                    op.ty.wgsl(),
                    emit::expr(sub, op)
                ));
            }
            (body, format!("v{}", sub.outputs.base_color))
        }
    };
    format!(
        "fn material_vertex_offset_graph(in: SurfaceInput, uv: vec2<f32>, time: f32, params: MaterialParams) -> vec3<f32> {{\n{body}    return {ret};\n}}\n"
    )
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
