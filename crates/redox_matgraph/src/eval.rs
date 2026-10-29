//! CPU interpreter for the IR. Reference semantics for constant folding and for GPU
//! comparisons of the generated WGSL (MAT-02 steps 2 and 4).

use crate::graph::Ty;
use crate::ir::{Ir, OpKind};

/// Runtime value: components in `c[0..components]`, the rest unused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value {
    /// Type of the value.
    pub ty: Ty,
    /// Components.
    pub c: [f32; 4],
}

impl Value {
    fn zero(ty: Ty) -> Self {
        Self { ty, c: [0.0; 4] }
    }
}

/// Inputs of one fragment for [`eval`].
#[derive(Clone, Debug, PartialEq)]
pub struct EvalInputs {
    /// Texture coordinates.
    pub uv: [f32; 2],
    /// World position.
    pub world_pos: [f32; 3],
    /// World normal.
    pub normal: [f32; 3],
    /// Normalised direction towards the camera.
    pub view_dir: [f32; 3],
    /// Per-instance colour.
    pub vertex_color: [f32; 4],
    /// Elapsed time.
    pub time: f32,
    /// Parameter values by slot; missing entries use the slot default.
    pub params: Vec<[f32; 4]>,
}

/// Surface properties produced by [`eval`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceValues {
    /// Base colour.
    pub base_color: [f32; 3],
    /// Opacity.
    pub opacity: f32,
    /// Shading normal.
    pub normal: [f32; 3],
    /// Metallic.
    pub metallic: f32,
    /// Roughness.
    pub roughness: f32,
    /// Emissive colour.
    pub emissive: [f32; 3],
}

/// Runs the IR on one fragment.
pub fn eval(ir: &Ir, inputs: &EvalInputs) -> SurfaceValues {
    let mut values: Vec<Value> = Vec::with_capacity(ir.ops.len());
    for op in &ir.ops {
        let args: Vec<Value> = op
            .args
            .iter()
            .map(|&a| values.get(a as usize).copied().unwrap_or(Value::zero(Ty::F32)))
            .collect();
        let value = match &op.kind {
            OpKind::Uv => Value {
                ty: Ty::Vec2,
                c: [inputs.uv[0], inputs.uv[1], 0.0, 0.0],
            },
            OpKind::WorldPos => vec3(Ty::Vec3, inputs.world_pos),
            OpKind::Normal => vec3(Ty::Vec3, inputs.normal),
            OpKind::ViewDir => vec3(Ty::Vec3, inputs.view_dir),
            OpKind::VertexColor => Value {
                ty: Ty::Vec4,
                c: inputs.vertex_color,
            },
            OpKind::Time => Value {
                ty: Ty::F32,
                c: [inputs.time, 0.0, 0.0, 0.0],
            },
            OpKind::Const(c) => Value { ty: op.ty, c: *c },
            OpKind::Param(slot) => {
                let fallback = ir.params.get(*slot).map_or([0.0; 4], |p| p.default);
                let c = inputs.params.get(*slot).copied().unwrap_or(fallback);
                Value { ty: op.ty, c }
            }
            kind => apply(kind, op.ty, &args).unwrap_or(Value::zero(op.ty)),
        };
        values.push(value);
    }
    let get = |id: u32| values.get(id as usize).copied().unwrap_or(Value::zero(Ty::F32));
    let outs = ir.outputs;
    let rgb = |v: Value| [v.c[0], v.c[1], v.c[2]];
    SurfaceValues {
        base_color: rgb(get(outs.base_color)),
        opacity: get(outs.opacity).c[0],
        normal: rgb(get(outs.normal)),
        metallic: get(outs.metallic).c[0],
        roughness: get(outs.roughness).c[0],
        emissive: rgb(get(outs.emissive)),
    }
}

fn vec3(ty: Ty, v: [f32; 3]) -> Value {
    Value {
        ty,
        c: [v[0], v[1], v[2], 0.0],
    }
}

/// Component `i` of `v`, with scalars broadcast.
fn component(v: &Value, i: usize) -> f32 {
    if v.ty.components() <= 1 {
        v.c[0]
    } else {
        v.c[i]
    }
}

/// Applies a pure operation to already evaluated operands. `ty` is the result type.
/// Returns `None` for operations that read inputs or parameters (not pure).
pub fn apply(kind: &OpKind, ty: Ty, args: &[Value]) -> Option<Value> {
    let value = match kind {
        OpKind::Add => map_components(ty, args, |x| x[0] + x[1]),
        OpKind::Sub => map_components(ty, args, |x| x[0] - x[1]),
        OpKind::Mul => map_components(ty, args, |x| x[0] * x[1]),
        OpKind::Pow => map_components(ty, args, |x| x[0].powf(x[1])),
        OpKind::OneMinus => map_components(ty, args, |x| 1.0 - x[0]),
        OpKind::Lerp => map_components(ty, args, |x| x[0] + (x[1] - x[0]) * x[2]),
        OpKind::Clamp => map_components(ty, args, |x| x[0].max(x[1]).min(x[2])),
        OpKind::Dot => {
            let first = args.first()?;
            let second = args.get(1)?;
            let sum = (0..first.ty.components())
                .map(|i| component(first, i) * component(second, i))
                .sum();
            Value {
                ty: Ty::F32,
                c: [sum, 0.0, 0.0, 0.0],
            }
        }
        OpKind::Normalize => {
            let x = args.first()?;
            let dim = ty.components();
            let len = (0..dim)
                .map(|i| component(x, i) * component(x, i))
                .sum::<f32>()
                .sqrt();
            let mut c = [0.0; 4];
            for (i, slot) in c.iter_mut().enumerate().take(dim) {
                *slot = if len > 0.0 { component(x, i) / len } else { component(x, i) };
            }
            Value { ty, c }
        }
        _ => return None,
    };
    Some(value)
}

/// Applies `f` to each component of the result, passing the operand components.
fn map_components(ty: Ty, args: &[Value], f: impl Fn(&[f32]) -> f32) -> Value {
    let dim = ty.components();
    let mut c = [0.0; 4];
    for (i, slot) in c.iter_mut().enumerate().take(dim) {
        let operands: Vec<f32> = args.iter().map(|a| component(a, i)).collect();
        *slot = f(&operands);
    }
    Value { ty, c }
}
