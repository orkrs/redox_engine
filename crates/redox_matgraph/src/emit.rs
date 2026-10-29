//! WGSL expression of one IR operation (MAT-02 step 3).
//!
//! Operands are named `v<id>` by the code generator. Scalars are splatted to the result type
//! where a WGSL builtin needs matching types; `+ - *` already accept scalar/vector mixes.

use crate::graph::Ty;
use crate::ir::{Ir, Op, OpKind};

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
