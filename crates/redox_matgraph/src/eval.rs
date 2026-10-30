//! CPU interpreter for the IR. Reference semantics for constant folding and for GPU
//! comparisons of the generated WGSL (MAT-02 steps 2 and 4, extended by MAT-03).
//!
//! Texture reads go through a [`TextureSampler`]; [`eval`] uses [`FlatTexture`], whose texel
//! is a flat tangent-space normal `(0.5, 0.5, 1)`. Custom WGSL nodes have no CPU meaning:
//! [`try_eval`] returns `None` for graphs that contain one.

use crate::graph::Ty;
use crate::ir::{Ir, MathFn, OpKind};
use crate::noise;
use crate::texture;

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

/// Inputs of one fragment for [`fn@eval`].
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

/// Surface properties produced by [`fn@eval`].
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

/// Source of texels for the interpreter.
pub trait TextureSampler {
    /// RGBA texel of texture `slot` at `uv`.
    fn sample(&self, slot: u8, uv: [f32; 2]) -> [f32; 4];
}

/// Sampler returning the flat tangent-space normal texel `(0.5, 0.5, 1, 1)` for every slot.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlatTexture;

impl TextureSampler for FlatTexture {
    fn sample(&self, _slot: u8, _uv: [f32; 2]) -> [f32; 4] {
        [0.5, 0.5, 1.0, 1.0]
    }
}

/// What a fragment evaluation can read besides [`EvalInputs`].
pub struct EvalEnv<'a> {
    /// Texel source.
    pub textures: &'a dyn TextureSampler,
    /// Surface tangent for normal maps. `None` derives one from the normal.
    pub tangent: Option<[f32; 3]>,
}

/// Runs the IR on one fragment with flat textures and a derived tangent.
pub fn eval(ir: &Ir, inputs: &EvalInputs) -> SurfaceValues {
    eval_with(
        ir,
        inputs,
        &EvalEnv {
            textures: &FlatTexture,
            tangent: None,
        },
    )
}

/// Like [`eval`], but returns `None` when the graph has a custom node (no CPU reference).
pub fn try_eval(ir: &Ir, inputs: &EvalInputs) -> Option<SurfaceValues> {
    let custom = ir
        .ops
        .iter()
        .any(|op| matches!(op.kind, OpKind::Custom { .. }));
    (!custom).then(|| eval(ir, inputs))
}

/// Runs the IR on one fragment with an explicit environment.
pub fn eval_with(ir: &Ir, inputs: &EvalInputs, env: &EvalEnv<'_>) -> SurfaceValues {
    let mut values: Vec<Value> = Vec::with_capacity(ir.ops.len());
    for op in &ir.ops {
        let args: Vec<Value> = op
            .args
            .iter()
            .map(|&a| {
                values
                    .get(a as usize)
                    .copied()
                    .unwrap_or(Value::zero(Ty::F32))
            })
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
            OpKind::Texture { slot, .. } => {
                let uv = [component(&args[0], 0), component(&args[0], 1)];
                Value {
                    ty: Ty::Vec4,
                    c: env.textures.sample(*slot, uv),
                }
            }
            OpKind::NormalMap { slot } => {
                let uv = [component(&args[0], 0), component(&args[0], 1)];
                let texel = env.textures.sample(*slot, uv);
                let n = inputs.normal;
                let t = env.tangent.unwrap_or_else(|| default_tangent(n));
                vec3(Ty::Vec3, texture::normal_map(texel, n, t))
            }
            OpKind::Triplanar { slot, scale } => triplanar(env, *slot, *scale, &args),
            kind => apply(kind, op.ty, &args).unwrap_or(Value::zero(op.ty)),
        };
        values.push(value);
    }
    let get = |id: u32| {
        values
            .get(id as usize)
            .copied()
            .unwrap_or(Value::zero(Ty::F32))
    };
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

/// Triplanar sample: the three axis projections blended with [`texture::triplanar_weights`].
fn triplanar(env: &EvalEnv<'_>, slot: u8, scale: f32, args: &[Value]) -> Value {
    let p = [
        component(&args[0], 0),
        component(&args[0], 1),
        component(&args[0], 2),
    ];
    let n = [
        component(&args[1], 0),
        component(&args[1], 1),
        component(&args[1], 2),
    ];
    let w = texture::triplanar_weights(n);
    let x = env.textures.sample(slot, [p[2] * scale, p[1] * scale]);
    let y = env.textures.sample(slot, [p[0] * scale, p[2] * scale]);
    let z = env.textures.sample(slot, [p[0] * scale, p[1] * scale]);
    let mut c = [0.0; 4];
    for (i, slot) in c.iter_mut().enumerate() {
        *slot = x[i] * w[0] + y[i] * w[1] + z[i] * w[2];
    }
    Value { ty: Ty::Vec4, c }
}

/// A tangent perpendicular to `n`, chosen deterministically; used when no tangent is given.
fn default_tangent(n: [f32; 3]) -> [f32; 3] {
    let helper = if n[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    [
        helper[1] * n[2] - helper[2] * n[1],
        helper[2] * n[0] - helper[0] * n[2],
        helper[0] * n[1] - helper[1] * n[0],
    ]
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
/// Returns `None` for operations that read inputs, parameters, textures or custom code.
pub fn apply(kind: &OpKind, ty: Ty, args: &[Value]) -> Option<Value> {
    let value = match kind {
        OpKind::Add => map_components(ty, args, |x| x[0] + x[1]),
        OpKind::Sub => map_components(ty, args, |x| x[0] - x[1]),
        OpKind::Mul => map_components(ty, args, |x| x[0] * x[1]),
        OpKind::Pow => map_components(ty, args, |x| x[0].powf(x[1])),
        OpKind::OneMinus => map_components(ty, args, |x| 1.0 - x[0]),
        OpKind::Lerp => map_components(ty, args, |x| x[0] + (x[1] - x[0]) * x[2]),
        OpKind::Clamp => map_components(ty, args, |x| x[0].max(x[1]).min(x[2])),
        OpKind::Math(f) => map_components(ty, args, |x| math(f, x[0])),
        OpKind::Min => map_components(ty, args, |x| x[0].min(x[1])),
        OpKind::Max => map_components(ty, args, |x| x[0].max(x[1])),
        OpKind::Step => map_components(ty, args, |x| if x[1] < x[0] { 0.0 } else { 1.0 }),
        OpKind::Smoothstep => map_components(ty, args, |x| smoothstep(x[0], x[1], x[2])),
        OpKind::Select => map_components(ty, args, |x| if x[0] >= 0.5 { x[1] } else { x[2] }),
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
        OpKind::Fresnel(power) => {
            let normal = args.first()?;
            let view = args.get(1)?;
            let d: f32 = (0..3)
                .map(|i| component(normal, i) * component(view, i))
                .sum();
            scalar((1.0 - d.clamp(0.0, 1.0)).powf(*power))
        }
        OpKind::Split(index) => scalar(component(args.first()?, usize::from(*index))),
        OpKind::Merge => Value {
            ty: Ty::Vec3,
            c: [
                args.first()?.c[0],
                args.get(1)?.c[0],
                args.get(2)?.c[0],
                0.0,
            ],
        },
        OpKind::Noise(kind) => {
            let p = args.first()?;
            let dims = p.ty.components() as u32;
            let point = [p.c[0], p.c[1], p.c[2]];
            scalar(noise::eval(*kind, point, dims))
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
                *slot = if len > 0.0 {
                    component(x, i) / len
                } else {
                    component(x, i)
                };
            }
            Value { ty, c }
        }
        _ => return None,
    };
    Some(value)
}

fn scalar(x: f32) -> Value {
    Value {
        ty: Ty::F32,
        c: [x, 0.0, 0.0, 0.0],
    }
}

/// Single-argument math; the remap order matches `emit::math`.
fn math(f: &MathFn, x: f32) -> f32 {
    match f {
        MathFn::Abs => x.abs(),
        MathFn::Floor => x.floor(),
        MathFn::Fract => x - x.floor(),
        MathFn::Saturate => x.clamp(0.0, 1.0),
        MathFn::Sin => x.sin(),
        MathFn::Cos => x.cos(),
        MathFn::Sqrt => x.sqrt(),
        MathFn::Remap([in_min, in_max, out_min, out_max]) => {
            ((x - in_min) / (in_max - in_min)) * (out_max - out_min) + out_min
        }
    }
}

/// `smoothstep(e0, e1, x)`; a degenerate edge pair (`e0 == e1`) acts as a step.
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 == e0 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
