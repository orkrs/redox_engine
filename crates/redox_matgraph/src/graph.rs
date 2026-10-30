//! Graph format: nodes, ports, value types and RON (de)serialisation (MAT-02 step 1, extended by
//! MAT-03). Checks live in [`crate::check`].

use serde::{Deserialize, Serialize};

/// Format version accepted by validation.
pub const FORMAT_VERSION: u32 = 1;

/// Texture slots a graph may reference (`tex0..tex3` in generated code).
pub const GRAPH_TEXTURE_SLOTS: u8 = 4;

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
    /// 2D texture (reserved; texture nodes read textures through their slot).
    Texture2d,
    /// Texture sampler (reserved).
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
    /// Sample of texture `slot` with the default sampler; input `uv`, output rgba.
    Texture2d {
        /// Texture slot; bound by the renderer.
        slot: u8,
    },
    /// Sample of texture `slot` with an explicit wrap mode (`clamp: true` clamps, otherwise
    /// repeats); input `uv`, output rgba.
    TextureSample {
        /// Texture slot.
        slot: u8,
        /// Clamp to edge instead of repeating.
        clamp: bool,
    },
    /// Tangent-space normal map in `slot`, decoded and rotated into world space by the surface
    /// normal and tangent; input `uv`, output `vec3`.
    NormalMap {
        /// Texture slot.
        slot: u8,
    },
    /// Triplanar sampling of texture `slot` along the world axes; inputs `position` and
    /// `normal` (`vec3`), output rgba.
    Triplanar {
        /// Texture slot.
        slot: u8,
        /// Texture repeats per metre.
        scale: f32,
    },
    /// Absolute value (component-wise).
    Abs,
    /// Floor (component-wise).
    Floor,
    /// `x - floor(x)` (component-wise).
    Fract,
    /// Clamp to `[0, 1]`.
    Saturate,
    /// Sine (component-wise).
    Sin,
    /// Cosine (component-wise).
    Cos,
    /// Square root (component-wise).
    Sqrt,
    /// Linear remap of `x` from `[in_min, in_max]` to `[out_min, out_max]`.
    Remap {
        /// Input range start.
        in_min: f32,
        /// Input range end; must differ from `in_min`.
        in_max: f32,
        /// Output range start.
        out_min: f32,
        /// Output range end.
        out_max: f32,
    },
    /// Component-wise minimum.
    Min,
    /// Component-wise maximum.
    Max,
    /// `step(edge, x)`: 0 below the edge, 1 otherwise.
    Step,
    /// `smoothstep(e0, e1, x)`.
    Smoothstep,
    /// Fresnel term `(1 - saturate(dot(normal, view)))^power`; inputs `normal`, `view` (`vec3`).
    Fresnel {
        /// Exponent.
        power: f32,
    },
    /// One component of a vector as `f32`.
    Split {
        /// Component index: 0 = x, 1 = y, 2 = z, 3 = w.
        component: u8,
    },
    /// Three scalars into a `vec3`.
    Merge,
    /// Selects input 1 when the scalar mask (input 0) is at least 0.5, else input 2.
    If,
    /// Value noise, input `position` (`vec2` or `vec3`), output `f32`.
    ValueNoise,
    /// Gradient noise, input `position` (`vec2` or `vec3`), output `f32`.
    GradientNoise,
    /// Worley F1 distance, input `position` (`vec2` or `vec3`), output `f32`.
    Worley,
    /// Custom WGSL function. The body is the function body; `inputs` are its parameters.
    Custom {
        /// Display name (not part of the generated code).
        name: String,
        /// Parameters in order, with their types.
        inputs: Vec<(String, Ty)>,
        /// Return type.
        output: Ty,
        /// Function body (WGSL statements, `return` included).
        body: String,
    },
    /// Blend of two colours by height: inputs `a`, `b` (`vec3`), `height_a`, `height_b` (`f32`).
    /// Weight of `a` is `saturate((height_a - height_b) * sharpness + 0.5)`.
    HeightBlend {
        /// Contrast of the blend; larger means a harder transition.
        sharpness: f32,
    },
    /// World-space normal bent by the screen-space derivatives of a height (`f32` input).
    /// Fragment stage only; the CPU interpreter returns the unbent normal.
    NormalFromHeight {
        /// Bump strength.
        strength: f32,
    },
    /// Sink of the vertex stage: world-space offset of the vertex in metres (`vec3`). Only
    /// vertex-safe nodes may feed it (no texture reads, no derivatives).
    PositionOffset {
        /// Longest offset the graph can produce; culling bounds grow by this much.
        max_offset: f32,
    },
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
const SURFACE_INPUTS: [Option<Ty>; 6] = [
    Some(Ty::Vec3),
    Some(Ty::F32),
    Some(Ty::Vec3),
    Some(Ty::F32),
    Some(Ty::F32),
    Some(Ty::Vec3),
];

/// Number of input ports of a node kind.
pub fn arity(kind: &NodeKind) -> usize {
    input_types(kind).len()
}

/// Input port types of a kind. `Some(ty)` fixes the port type; `None` marks an arithmetic
/// operand, whose type is joined with the other arithmetic operands (scalars broadcast).
pub fn input_types(kind: &NodeKind) -> Vec<Option<Ty>> {
    use NodeKind as K;
    match kind {
        K::Add | K::Sub | K::Mul | K::Dot | K::Pow | K::Min | K::Max | K::Step => vec![None; 2],
        K::Lerp | K::Clamp | K::Smoothstep => vec![None; 3],
        K::Normalize
        | K::OneMinus
        | K::Abs
        | K::Floor
        | K::Fract
        | K::Saturate
        | K::Sin
        | K::Cos
        | K::Sqrt
        | K::Remap { .. }
        | K::Split { .. }
        | K::ValueNoise
        | K::GradientNoise
        | K::Worley => vec![None],
        K::Texture2d { .. } | K::TextureSample { .. } | K::NormalMap { .. } => {
            vec![Some(Ty::Vec2)]
        }
        K::Triplanar { .. } | K::Fresnel { .. } => vec![Some(Ty::Vec3), Some(Ty::Vec3)],
        K::HeightBlend { .. } => vec![Some(Ty::Vec3), Some(Ty::Vec3), Some(Ty::F32), Some(Ty::F32)],
        K::NormalFromHeight { .. } => vec![Some(Ty::F32)],
        K::PositionOffset { .. } => vec![Some(Ty::Vec3)],
        K::Merge => vec![Some(Ty::F32); 3],
        K::If => vec![Some(Ty::F32), None, None],
        K::Custom { inputs, .. } => inputs.iter().map(|(_, ty)| Some(*ty)).collect(),
        K::SurfaceOutput => SURFACE_INPUTS.to_vec(),
        _ => Vec::new(),
    }
}

/// Output type of a kind given the actual (already checked) input types.
pub fn output_type(kind: &NodeKind, inputs: &[Ty]) -> Ty {
    use NodeKind as K;
    match kind {
        K::Uv => Ty::Vec2,
        K::WorldPos
        | K::Normal
        | K::ViewDir
        | K::ConstVec3(_)
        | K::Merge
        | K::NormalMap { .. }
        | K::HeightBlend { .. }
        | K::NormalFromHeight { .. }
        | K::PositionOffset { .. } => Ty::Vec3,
        K::VertexColor => Ty::Vec4,
        K::Time
        | K::ConstF32(_)
        | K::Dot
        | K::SurfaceOutput
        | K::Fresnel { .. }
        | K::Split { .. }
        | K::ValueNoise
        | K::GradientNoise
        | K::Worley => Ty::F32,
        K::Texture2d { .. } | K::TextureSample { .. } | K::Triplanar { .. } => Ty::Vec4,
        K::Param { ty, .. } => *ty,
        K::Custom { output, .. } => *output,
        K::Normalize
        | K::OneMinus
        | K::Abs
        | K::Floor
        | K::Fract
        | K::Saturate
        | K::Sin
        | K::Cos
        | K::Sqrt
        | K::Remap { .. } => inputs.first().copied().unwrap_or(Ty::F32),
        K::Add
        | K::Sub
        | K::Mul
        | K::Lerp
        | K::Clamp
        | K::Pow
        | K::Min
        | K::Max
        | K::Step
        | K::Smoothstep
        | K::If => join(inputs),
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
