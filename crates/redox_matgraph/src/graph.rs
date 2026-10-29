//! Graph format: nodes, ports, value types and RON (de)serialisation (MAT-02 step 1).
//! Checks live in [`crate::check`].

use serde::{Deserialize, Serialize};

/// Format version accepted by validation.
pub const FORMAT_VERSION: u32 = 1;

/// Names of the six inputs of [`NodeKind::SurfaceOutput`], in port order.
pub const SURFACE_PORT_NAMES: [&str; 6] = [
    "base_color",
    "opacity",
    "normal",
    "metallic",
    "roughness",
    "emissive",
];

/// Value types of graph ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ty {
    /// 32-bit float scalar.
    F32,
    /// Two-component float vector.
    Vec2,
    /// Three-component float vector.
    Vec3,
    /// Four-component float vector.
    Vec4,
    /// 2D texture (reserved for MAT-03).
    Texture2d,
    /// Texture sampler (reserved for MAT-03).
    Sampler,
}

impl Ty {
    /// Number of components; 1 for scalars, 0 for resources.
    pub fn components(self) -> usize {
        match self {
            Ty::F32 => 1,
            Ty::Vec2 => 2,
            Ty::Vec3 => 3,
            Ty::Vec4 => 4,
            Ty::Texture2d | Ty::Sampler => 0,
        }
    }

    /// Whether the type is `vec2`, `vec3` or `vec4`.
    pub fn is_vector(self) -> bool {
        matches!(self, Ty::Vec2 | Ty::Vec3 | Ty::Vec4)
    }

    /// WGSL spelling of the type.
    pub fn wgsl(self) -> &'static str {
        match self {
            Ty::F32 => "f32",
            Ty::Vec2 => "vec2<f32>",
            Ty::Vec3 => "vec3<f32>",
            Ty::Vec4 => "vec4<f32>",
            Ty::Texture2d => "texture_2d<f32>",
            Ty::Sampler => "sampler",
        }
    }
}

/// Identifier of a node inside one graph.
pub type NodeId = u32;

/// Reference to the single output (port 0) of another node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortRef {
    /// Source node.
    pub node: NodeId,
    /// Output port of the source; only 0 exists in format version 1.
    pub port: u8,
}

/// What a node does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Interpolated texture coordinates (`vec2`).
    Uv,
    /// World-space position (`vec3`).
    WorldPos,
    /// World-space normal (`vec3`).
    Normal,
    /// Normalised direction towards the camera (`vec3`).
    ViewDir,
    /// Per-instance colour (`vec4`).
    VertexColor,
    /// Elapsed time in seconds (`f32`).
    Time,
    /// Scalar constant.
    ConstF32(f32),
    /// Vector constant.
    ConstVec3([f32; 3]),
    /// Per-instance parameter with a default; stored in the instance uniform.
    Param {
        /// Unique identifier of the parameter.
        name: String,
        /// Arithmetic type of the parameter.
        ty: Ty,
        /// Default value (`.x` for scalars, `.xy`/`.xyz` for shorter vectors).
        default: [f32; 4],
    },
    /// Component-wise addition with scalar broadcasting.
    Add,
    /// Component-wise subtraction with scalar broadcasting.
    Sub,
    /// Component-wise multiplication with scalar broadcasting.
    Mul,
    /// Linear interpolation `mix(a, b, t)`.
    Lerp,
    /// Clamp `x` to `[min, max]`.
    Clamp,
    /// Dot product of two vectors (`f32`).
    Dot,
    /// Vector normalisation.
    Normalize,
    /// Component-wise power.
    Pow,
    /// `1 - x`.
    OneMinus,
    /// Sink of the graph: the six surface properties.
    SurfaceOutput,
}

/// One graph node. `inputs` are positional; `None` means "no link".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Unique id.
    pub id: NodeId,
    /// Operation of the node.
    pub kind: NodeKind,
    /// Input links, one per port.
    pub inputs: Vec<Option<PortRef>>,
}

/// A material graph as stored in `.matgraph` files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraph {
    /// Format version; must equal [`FORMAT_VERSION`].
    pub version: u32,
    /// Human-readable name (not part of the generated code).
    pub name: String,
    /// Nodes in any order.
    pub nodes: Vec<Node>,
}

/// Error found while loading or checking a graph. `message` already names the node.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct GraphError {
    /// Node the error belongs to, if any.
    pub node: Option<NodeId>,
    /// Full message, including the node id when present.
    pub message: String,
}

impl GraphError {
    /// Builds an error, prefixing the message with the node id when one is given.
    pub fn new(node: Option<NodeId>, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        let message = match node {
            Some(id) => format!("узел {id}: {detail}"),
            None => detail,
        };
        Self { node, message }
    }
}

/// Input types of the surface output, in port order.
const SURFACE_INPUTS: &[Option<Ty>] = &[
    Some(Ty::Vec3),
    Some(Ty::F32),
    Some(Ty::Vec3),
    Some(Ty::F32),
    Some(Ty::F32),
    Some(Ty::Vec3),
];
const NO_INPUTS: &[Option<Ty>] = &[];
const ARITY_ONE: &[Option<Ty>] = &[None];
const ARITY_TWO: &[Option<Ty>] = &[None, None];
const ARITY_THREE: &[Option<Ty>] = &[None, None, None];

/// Number of input ports of a node kind.
pub fn arity(kind: &NodeKind) -> usize {
    input_types(kind).len()
}

/// Input port types of a kind. `Some(ty)` fixes the port type; `None` marks an arithmetic
/// operand, whose type is joined with the other arithmetic operands (scalars broadcast).
pub fn input_types(kind: &NodeKind) -> &'static [Option<Ty>] {
    match kind {
        NodeKind::Add | NodeKind::Sub | NodeKind::Mul | NodeKind::Dot | NodeKind::Pow => ARITY_TWO,
        NodeKind::Lerp | NodeKind::Clamp => ARITY_THREE,
        NodeKind::Normalize | NodeKind::OneMinus => ARITY_ONE,
        NodeKind::SurfaceOutput => SURFACE_INPUTS,
        _ => NO_INPUTS,
    }
}

/// Output type of a kind given the actual (already checked) input types.
pub fn output_type(kind: &NodeKind, inputs: &[Ty]) -> Ty {
    match kind {
        NodeKind::Uv => Ty::Vec2,
        NodeKind::WorldPos | NodeKind::Normal | NodeKind::ViewDir | NodeKind::ConstVec3(_) => {
            Ty::Vec3
        }
        NodeKind::VertexColor => Ty::Vec4,
        NodeKind::Time | NodeKind::ConstF32(_) | NodeKind::Dot | NodeKind::SurfaceOutput => Ty::F32,
        NodeKind::Param { ty, .. } => *ty,
        NodeKind::Normalize | NodeKind::OneMinus => inputs.first().copied().unwrap_or(Ty::F32),
        NodeKind::Add
        | NodeKind::Sub
        | NodeKind::Mul
        | NodeKind::Lerp
        | NodeKind::Clamp
        | NodeKind::Pow => join(inputs),
    }
}

/// Result type of arithmetic over `inputs`: the vector type if any operand is a vector,
/// otherwise `f32`.
fn join(inputs: &[Ty]) -> Ty {
    inputs
        .iter()
        .copied()
        .find(|t| *t != Ty::F32)
        .unwrap_or(Ty::F32)
}

impl MaterialGraph {
    /// Parses a graph from RON text. Only syntax and shape are checked here; call
    /// [`MaterialGraph::validate`] for semantic checks.
    pub fn from_ron(text: &str) -> Result<Self, GraphError> {
        ron::from_str(text)
            .map_err(|e| GraphError::new(None, format!("не удалось разобрать граф: {e}")))
    }

    /// Serialises the graph to RON with nodes sorted by id, so equal graphs give equal text.
    pub fn to_ron(&self) -> String {
        let mut sorted = self.clone();
        sorted.nodes.sort_by_key(|node| node.id);
        ron::ser::to_string_pretty(&sorted, ron::ser::PrettyConfig::default()).unwrap_or_default()
    }
}
