//! WGSL expression of one IR operation (MAT-02 step 3, extended by MAT-03).
//!
//! Operands are named `v<id>` by the code generator. Scalars are splatted to the result type
//! where a WGSL builtin needs matching types; `+ - *` already accept scalar/vector mixes.

use crate::graph::Ty;
use crate::ir::{Ir, MathFn, Op, OpKind, SamplerKind};

/// WGSL expression computing `op` from its operands.
pub(crate) fn expr(ir: &Ir, op: &Op) -> String {
    let arg = |i: usize| -> String {
        op.args
            .get(i)
            .map_or_else(|| "0.0".to_string(), |a| format!("v{a}"))
    };
    let arg_ty = |i: usize| -> Ty {
        op.args
            .get(i)
            .and_then(|a| ir.ops.get(*a as usize))
            .map_or(Ty::F32, |o| o.ty)
    };
    let splat_arg = |i: usize| splat(&arg(i), arg_ty(i), op.ty);
    match &op.kind {
        OpKind::Uv => "uv".to_string(),
        OpKind::WorldPos => "in.world_pos".to_string(),
        OpKind::Normal => "in.world_normal".to_string(),
        OpKind::ViewDir => "normalize(in.to_camera)".to_string(),
        OpKind::VertexColor => "in.instance_color".to_string(),
        OpKind::Time => "time".to_string(),
        OpKind::Const(c) => constant(op.ty, c),
        OpKind::Param(slot) => param(ir, *slot, op.ty),
        OpKind::Add => format!("({} + {})", arg(0), arg(1)),
        OpKind::Sub => format!("({} - {})", arg(0), arg(1)),
        OpKind::Mul => format!("({} * {})", arg(0), arg(1)),
        OpKind::Lerp => format!(
            "mix({}, {}, {})",
            splat_arg(0),
            splat(&arg(1), arg_ty(1), op.ty),
            arg(2)
        ),
        OpKind::Clamp => format!(
            "clamp({}, {}, {})",
            splat_arg(0),
            splat(&arg(1), arg_ty(1), op.ty),
            splat(&arg(2), arg_ty(2), op.ty)
        ),
        OpKind::Dot => format!("dot({}, {})", arg(0), arg(1)),
        OpKind::Normalize => format!("normalize({})", arg(0)),
        OpKind::Pow => format!(
            "pow({}, {})",
            splat_arg(0),
            splat(&arg(1), arg_ty(1), op.ty)
        ),
        OpKind::OneMinus => format!("(1.0 - {})", arg(0)),
        OpKind::Texture { slot, sampler } => {
            format!(
                "textureSample(tex{slot}, {}, {})",
                sampler_name(*sampler),
                arg(0)
            )
        }
        OpKind::NormalMap { slot } => format!(
            "mg_normal_map(tex{slot}, samp_default, {}, in.world_normal, in.world_tangent)",
            arg(0)
        ),
        OpKind::Triplanar { slot, scale } => format!(
            "mg_triplanar(tex{slot}, samp_default, {}, {}, {})",
            arg(0),
            arg(1),
            literal(*scale)
        ),
        OpKind::Math(f) => math(f, &arg(0)),
        OpKind::Min => format!("min({}, {})", splat_arg(0), splat_arg(1)),
        OpKind::Max => format!("max({}, {})", splat_arg(0), splat_arg(1)),
        OpKind::Step => format!("step({}, {})", splat_arg(0), splat_arg(1)),
        OpKind::Smoothstep => format!(
            "smoothstep({}, {}, {})",
            splat_arg(0),
            splat_arg(1),
            splat_arg(2)
        ),
        OpKind::Select => format!(
            "select({}, {}, {} >= 0.5)",
            splat(&arg(2), arg_ty(2), op.ty),
            splat(&arg(1), arg_ty(1), op.ty),
            arg(0)
        ),
        OpKind::Fresnel(power) => format!(
            "pow(1.0 - saturate(dot({}, {})), {})",
            arg(0),
            arg(1),
            literal(*power)
        ),
        OpKind::Split(index) => {
            let swizzle = ["x", "y", "z", "w"]
                .get(usize::from(*index))
                .copied()
                .unwrap_or("x");
            format!("{}.{swizzle}", arg(0))
        }
        OpKind::Merge => format!("vec3<f32>({}, {}, {})", arg(0), arg(1), arg(2)),
        OpKind::Noise(kind) => {
            let dims = arg_ty(0).components();
            let point = if arg_ty(0) == Ty::Vec2 {
                format!("vec3<f32>({}, 0.0)", arg(0))
            } else {
                arg(0)
            };
            format!("{}({point}, {dims}u)", kind.wgsl_fn())
        }
        OpKind::Custom { node, inputs, .. } => {
            let args: Vec<String> = (0..inputs.len()).map(arg).collect();
            format!("custom_{node}({})", args.join(", "))
        }
    }
}

/// WGSL name of a sampler binding.
fn sampler_name(sampler: SamplerKind) -> &'static str {
    match sampler {
        SamplerKind::Default => "samp_default",
        SamplerKind::Repeat => "samp_repeat",
        SamplerKind::Clamp => "samp_clamp",
    }
}

/// Single-argument math. The remap formula matches [`crate::eval`] operation by operation.
fn math(f: &MathFn, x: &str) -> String {
    match f {
        MathFn::Abs => format!("abs({x})"),
        MathFn::Floor => format!("floor({x})"),
        MathFn::Fract => format!("fract({x})"),
        MathFn::Saturate => format!("saturate({x})"),
        MathFn::Sin => format!("sin({x})"),
        MathFn::Cos => format!("cos({x})"),
        MathFn::Sqrt => format!("sqrt({x})"),
        MathFn::Remap([in_min, in_max, out_min, out_max]) => format!(
            "((({x} - {}) / ({} - {})) * ({} - {}) + {})",
            literal(*in_min),
            literal(*in_max),
            literal(*in_min),
            literal(*out_max),
            literal(*out_min),
            literal(*out_min)
        ),
    }
}

/// `name` converted to `to` when it is a scalar feeding a vector builtin.
fn splat(name: &str, from: Ty, to: Ty) -> String {
    if from == Ty::F32 && to != Ty::F32 {
        format!("{}({name})", to.wgsl())
    } else {
        name.to_string()
    }
}

/// Literal of a finite `f32`; non-finite values (rejected by validation) become `0.0`.
fn literal(x: f32) -> String {
    if x.is_finite() {
        format!("{x:?}")
    } else {
        "0.0".to_string()
    }
}

fn constant(ty: Ty, c: &[f32; 4]) -> String {
    match ty {
        Ty::F32 | Ty::Texture2d | Ty::Sampler => literal(c[0]),
        Ty::Vec2 => format!("vec2<f32>({}, {})", literal(c[0]), literal(c[1])),
        Ty::Vec3 => format!(
            "vec3<f32>({}, {}, {})",
            literal(c[0]),
            literal(c[1]),
            literal(c[2])
        ),
        Ty::Vec4 => format!(
            "vec4<f32>({}, {}, {}, {})",
            literal(c[0]),
            literal(c[1]),
            literal(c[2]),
            literal(c[3])
        ),
    }
}

/// Read of parameter `slot` from the `MaterialParams` uniform, swizzled to its type.
fn param(ir: &Ir, slot: usize, ty: Ty) -> String {
    let name = ir.params.get(slot).map_or("", |p| p.name.as_str());
    let swizzle = match ty {
        Ty::F32 => ".x",
        Ty::Vec2 => ".xy",
        Ty::Vec3 => ".xyz",
        _ => "",
    };
    format!("params.p_{name}{swizzle}")
}
