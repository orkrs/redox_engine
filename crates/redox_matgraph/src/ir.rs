//! SSA intermediate representation: lowering, optimisation and dead-code removal
//! (MAT-02 step 2).

use std::collections::HashMap;

use crate::check::node_types;
use crate::eval::{Value, apply};
use crate::graph::{GraphError, MaterialGraph, Node, NodeId, NodeKind, Ty, arity};

/// Operation of the IR. Arguments are carried by [`Op::args`].
#[derive(Clone, Debug, PartialEq)]
pub enum OpKind {
    /// Texture coordinates.
    Uv,
    /// World position.
    WorldPos,
    /// World normal.
    Normal,
    /// Normalised direction towards the camera.
    ViewDir,
    /// Per-instance colour.
    VertexColor,
    /// Elapsed time.
    Time,
    /// Constant; the value is stored in the first components as needed by [`Op::ty`].
    Const([f32; 4]),
    /// Read of a parameter; the index refers to [`Ir::params`].
    Param(usize),
    /// `a + b`.
    Add,
    /// `a - b`.
    Sub,
    /// `a * b`.
    Mul,
    /// `mix(a, b, t)`.
    Lerp,
    /// `clamp(x, lo, hi)`.
    Clamp,
    /// `dot(a, b)`.
    Dot,
    /// `normalize(x)`.
    Normalize,
    /// `pow(x, e)`.
    Pow,
    /// `1 - x`.
    OneMinus,
}

/// One SSA operation. `id` equals its position in [`Ir::ops`]; every argument is an earlier id.
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    /// Position of the operation.
    pub id: u32,
    /// What the operation computes.
    pub kind: OpKind,
    /// Ids of the operands.
    pub args: Vec<u32>,
    /// Type of the result.
    pub ty: Ty,
}

/// Slot of a parameter in the instance uniform. Each slot is one `vec4` (16 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSlot {
    /// Parameter name.
    pub name: String,
    /// Arithmetic type of the parameter.
    pub ty: Ty,
    /// Offset in `f32` units inside the instance uniform (`4 * slot index`).
    pub offset: u32,
    /// Default value.
    pub default: [f32; 4],
}

/// Op ids of the six surface properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceOutputs {
    /// Base colour (`vec3`).
    pub base_color: u32,
    /// Opacity (`f32`).
    pub opacity: u32,
    /// Shading normal (`vec3`).
    pub normal: u32,
    /// Metallic (`f32`).
    pub metallic: u32,
    /// Roughness (`f32`).
    pub roughness: u32,
    /// Emissive colour (`vec3`).
    pub emissive: u32,
}

impl SurfaceOutputs {
    /// The six op ids in port order.
    pub fn as_array(&self) -> [u32; 6] {
        [
            self.base_color,
            self.opacity,
            self.normal,
            self.metallic,
            self.roughness,
            self.emissive,
        ]
    }

    /// Applies `f` to every op id.
    pub fn map(self, f: impl Fn(u32) -> u32) -> Self {
        Self {
            base_color: f(self.base_color),
            opacity: f(self.opacity),
            normal: f(self.normal),
            metallic: f(self.metallic),
            roughness: f(self.roughness),
            emissive: f(self.emissive),
        }
    }
}

/// A lowered material: operations in dependency order, parameter slots and outputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Ir {
    /// Operations, arguments first.
    pub ops: Vec<Op>,
    /// Parameter slots, sorted by name.
    pub params: Vec<ParamSlot>,
    /// Surface outputs.
    pub outputs: SurfaceOutputs,
}

/// Default operation used for an unconnected surface port.
fn surface_default(port: usize) -> (OpKind, Ty) {
    match port {
        0 => (OpKind::Const([0.8, 0.8, 0.8, 0.0]), Ty::Vec3),
        1 => (OpKind::Const([1.0, 0.0, 0.0, 0.0]), Ty::F32),
        2 => (OpKind::Normal, Ty::Vec3),
        3 => (OpKind::Const([0.0; 4]), Ty::F32),
        4 => (OpKind::Const([0.5, 0.0, 0.0, 0.0]), Ty::F32),
        _ => (OpKind::Const([0.0; 4]), Ty::Vec3),
    }
}

/// Validates `graph` and lowers it to SSA. Only nodes reachable from the surface are kept.
pub fn lower(graph: &MaterialGraph) -> Result<Ir, Vec<GraphError>> {
    let types = node_types(graph)?;
    let index: HashMap<NodeId, &Node> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let (params, slots) = param_slots(graph);
    let surface = graph
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::SurfaceOutput))
        .ok_or_else(|| vec![GraphError::new(None, "нет узла SurfaceOutput")])?;
    let mut lowering = Lowering {
        index,
        types,
        slots,
        ops: Vec::new(),
        done: HashMap::new(),
    };
    let mut args = Vec::with_capacity(6);
    for port in 0..arity(&surface.kind) {
        let arg = match surface.inputs.get(port).cloned().flatten() {
            Some(link) => lowering.value(link.node),
            None => lowering.default_op(port),
        };
        args.push(arg);
    }
    let outputs = SurfaceOutputs {
        base_color: args[0],
        opacity: args[1],
        normal: args[2],
        metallic: args[3],
        roughness: args[4],
        emissive: args[5],
    };
    Ok(Ir {
        ops: lowering.ops,
        params,
        outputs,
    })
}

/// Parameter slots sorted by name, and the name -> slot index map.
fn param_slots(graph: &MaterialGraph) -> (Vec<ParamSlot>, HashMap<String, usize>) {
    let mut found: Vec<(&str, Ty, [f32; 4])> = graph
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Param { name, ty, default } => Some((name.as_str(), *ty, *default)),
            _ => None,
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(b.0));
    let slots = found
        .iter()
        .enumerate()
        .map(|(i, (name, ty, default))| ParamSlot {
            name: (*name).to_string(),
            ty: *ty,
            offset: (i * 4) as u32,
            default: *default,
        })
        .collect();
    let map = found
        .iter()
        .enumerate()
        .map(|(i, (name, _, _))| ((*name).to_string(), i))
        .collect();
    (slots, map)
}

/// Emits operations depth-first, operands before users.
struct Lowering<'a> {
    index: HashMap<NodeId, &'a Node>,
    types: HashMap<NodeId, Ty>,
    slots: HashMap<String, usize>,
    ops: Vec<Op>,
    done: HashMap<NodeId, u32>,
}

impl Lowering<'_> {
    fn push(&mut self, kind: OpKind, ty: Ty, args: Vec<u32>) -> u32 {
        let id = self.ops.len() as u32;
        self.ops.push(Op { id, kind, args, ty });
        id
    }

    fn default_op(&mut self, port: usize) -> u32 {
        let (kind, ty) = surface_default(port);
        self.push(kind, ty, Vec::new())
    }

    fn value(&mut self, id: NodeId) -> u32 {
        if let Some(&op) = self.done.get(&id) {
            return op;
        }
        // Node ids were checked by `node_types`; a missing node is unreachable here.
        let Some(node) = self.index.get(&id).copied() else {
            return self.push(OpKind::Const([0.0; 4]), Ty::F32, Vec::new());
        };
        let mut args = Vec::new();
        for port in 0..arity(&node.kind) {
            let arg = match node.inputs.get(port).cloned().flatten() {
                Some(link) => self.value(link.node),
                None => self.default_op(port),
            };
            args.push(arg);
        }
        let ty = self.types.get(&id).copied().unwrap_or(Ty::F32);
        let kind = self.kind_of(&node.kind);
        let op = self.push(kind, ty, args);
        self.done.insert(id, op);
        op
    }

    fn kind_of(&self, kind: &NodeKind) -> OpKind {
        match kind {
            NodeKind::Uv => OpKind::Uv,
            NodeKind::WorldPos => OpKind::WorldPos,
            NodeKind::Normal => OpKind::Normal,
            NodeKind::ViewDir => OpKind::ViewDir,
            NodeKind::VertexColor => OpKind::VertexColor,
            NodeKind::Time => OpKind::Time,
            NodeKind::ConstF32(v) => OpKind::Const([*v, 0.0, 0.0, 0.0]),
            NodeKind::ConstVec3(v) => OpKind::Const([v[0], v[1], v[2], 0.0]),
            NodeKind::Param { name, .. } => {
                OpKind::Param(self.slots.get(name).copied().unwrap_or(0))
            }
            NodeKind::Add => OpKind::Add,
            NodeKind::Sub => OpKind::Sub,
            NodeKind::Mul => OpKind::Mul,
            NodeKind::Lerp => OpKind::Lerp,
            NodeKind::Clamp => OpKind::Clamp,
            NodeKind::Dot => OpKind::Dot,
            NodeKind::Normalize => OpKind::Normalize,
            NodeKind::Pow => OpKind::Pow,
            NodeKind::OneMinus => OpKind::OneMinus,
            NodeKind::SurfaceOutput => OpKind::Const([0.0; 4]),
        }
    }
}

/// Folds constants, shares identical operations, and removes unreachable ones.
pub fn optimize(ir: Ir) -> Ir {
    let Ir {
        ops,
        params,
        outputs,
    } = ir;
    let mut folded: Vec<Op> = Vec::with_capacity(ops.len());
    let mut remap: Vec<u32> = Vec::with_capacity(ops.len());
    let mut seen: HashMap<String, u32> = HashMap::new();
    for op in ops {
        let args: Vec<u32> = op
            .args
            .iter()
            .map(|&a| remap.get(a as usize).copied().unwrap_or(0))
            .collect();
        let (kind, args) = match fold(&op.kind, &args, &folded, op.ty) {
            Some(value) => (OpKind::Const(value), Vec::new()),
            None => (op.kind, args),
        };
        let key = format!("{kind:?}|{args:?}|{:?}", op.ty);
        let id = match seen.get(&key) {
            Some(&id) => id,
            None => {
                let id = folded.len() as u32;
                seen.insert(key, id);
                folded.push(Op {
                    id,
                    kind,
                    args,
                    ty: op.ty,
                });
                id
            }
        };
        remap.push(id);
    }
    let roots = outputs.map(|r| remap.get(r as usize).copied().unwrap_or(0));
    prune(Ir {
        ops: folded,
        params,
        outputs: roots,
    })
}

/// Value of a pure operation when all operands are constants.
fn fold(kind: &OpKind, args: &[u32], ops: &[Op], ty: Ty) -> Option<[f32; 4]> {
    if args.is_empty() || !is_pure(kind) {
        return None;
    }
    let mut values = Vec::with_capacity(args.len());
    for &a in args {
        let op = ops.get(a as usize)?;
        match &op.kind {
            OpKind::Const(c) => values.push(Value { ty: op.ty, c: *c }),
            _ => return None,
        }
    }
    apply(kind, ty, &values).map(|v| v.c)
}

/// Operations whose value depends only on their operands.
fn is_pure(kind: &OpKind) -> bool {
    !matches!(
        kind,
        OpKind::Uv
            | OpKind::WorldPos
            | OpKind::Normal
            | OpKind::ViewDir
            | OpKind::VertexColor
            | OpKind::Time
            | OpKind::Const(_)
            | OpKind::Param(_)
    )
}

/// Removes operations that do not contribute to a surface output and renumbers the rest.
pub fn prune(ir: Ir) -> Ir {
    let Ir {
        ops,
        params,
        outputs,
    } = ir;
    let mut live = vec![false; ops.len()];
    let mut stack: Vec<u32> = outputs.as_array().to_vec();
    while let Some(id) = stack.pop() {
        let Some(flag) = live.get_mut(id as usize) else {
            continue;
        };
        if *flag {
            continue;
        }
        *flag = true;
        if let Some(op) = ops.get(id as usize) {
            stack.extend(op.args.iter().copied());
        }
    }
    let mut renumber = vec![0u32; ops.len()];
    let mut kept: Vec<Op> = Vec::new();
    for (old, op) in ops.into_iter().enumerate() {
        if live.get(old).copied().unwrap_or(false) {
            renumber[old] = kept.len() as u32;
            kept.push(op);
        }
    }
    for (new, op) in kept.iter_mut().enumerate() {
        op.id = new as u32;
        op.args = op
            .args
            .iter()
            .map(|&a| renumber.get(a as usize).copied().unwrap_or(0))
            .collect();
    }
    let outputs = outputs.map(|r| renumber.get(r as usize).copied().unwrap_or(0));
    Ir {
        ops: kept,
        params,
        outputs,
    }
}

/// Copy of `ir` whose only live output is opacity (used by depth and shadow variants).
pub fn opacity_only(ir: &Ir) -> Ir {
    let opacity = ir.outputs.opacity;
    let outputs = SurfaceOutputs {
        base_color: opacity,
        opacity,
        normal: opacity,
        metallic: opacity,
        roughness: opacity,
        emissive: opacity,
    };
    prune(Ir {
        ops: ir.ops.clone(),
        params: ir.params.clone(),
        outputs,
    })
}
