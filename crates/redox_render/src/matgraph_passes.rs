//! Graph variants of the non-forward passes (MAT-02 step 5): transparent (alpha and additive),
//! normal prepass and point-light shadow atlas.
//!
//! The forward and transparent variants reuse the PBR module of [`crate::matgraph_cache`]. The
//! prepass and shadow variants are small modules of their own: they keep the geometry of the
//! pass they replace and only evaluate the graph opacity, discarding fragments below
//! [`ALPHA_CUTOFF`]. Slots without a graph never reach this module.

use redox_matgraph::{Generated, Pass};

use crate::matgraph_cache::{GRAPH_PARAMS_GROUP, MaterialGraphs};
use crate::mesh::CompressedVertex;
use crate::pass::forward::DEPTH_FORMAT;
use crate::pass::normal::SURFACE_FORMAT as NORMAL_SURFACE_FORMAT;
use crate::pass::shadow::SHADOW_FORMAT;
use crate::pass::shared_layouts::multisample_state;
use crate::shader::{MATERIAL_SURFACE_WGSL, shader_with_vertex_decode};

/// Colour format of the normal prepass target. ASSUMPTION: matches `NormalPass::new` in
/// `renderer/init.rs` (`Rgba16Float`).
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Graph opacity below which a fragment of a depth or shadow variant is discarded.
pub const ALPHA_CUTOFF: f32 = 0.5;

/// Render pass a graph variant is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphPass {
    /// Opaque forward PBR pass.
    Forward,
    /// Alpha-blended transparent pass.
    Alpha,
    /// Additive transparent pass.
    Additive,
    /// Normal and depth prepass (alpha cut-out only).
    Depth,
    /// Point-light shadow atlas (alpha cut-out only).
    Shadow,
    /// Virtual shadow map pages of the sun (alpha cut-out only).
    VsmDepth,
    /// Visibility buffer resolve of virtual geometry (MAT-07 step 2): the surface of the graph
    /// replaces the base shading of the pixels of its slot.
    VgResolve,
}

impl GraphPass {
    /// Every pass, in the order pipelines are built.
    pub const ALL: [GraphPass; 7] = [
        GraphPass::Forward,
        GraphPass::Alpha,
        GraphPass::Additive,
        GraphPass::Depth,
        GraphPass::Shadow,
        GraphPass::VsmDepth,
        GraphPass::VgResolve,
    ];

    /// Codegen pass whose WGSL this pass uses. Alpha and additive share the forward module.
    pub fn codegen(self) -> Pass {
        match self {
            GraphPass::Forward | GraphPass::Alpha | GraphPass::Additive | GraphPass::VgResolve => {
                Pass::Main
            }
            GraphPass::Depth => Pass::Depth,
            GraphPass::Shadow | GraphPass::VsmDepth => Pass::Shadow,
        }
    }

    /// Codegen features of this pass. The sun's shadow pages are drawn instanced, which also
    /// keeps their module key apart from the point-light atlas variant (same IR, other template).
    /// The VG resolve is keyed apart by `skinning` for the same reason.
    pub fn features(self) -> redox_matgraph::Features {
        redox_matgraph::Features {
            instanced: self == GraphPass::VsmDepth,
            skinning: self == GraphPass::VgResolve,
            ..redox_matgraph::Features::default()
        }
    }

    /// Bind group that holds the graph parameter block in this pass's pipeline layout.
    pub fn params_group(self) -> u32 {
        match self {
            GraphPass::Forward | GraphPass::Alpha | GraphPass::Additive => GRAPH_PARAMS_GROUP,
            GraphPass::Depth | GraphPass::VsmDepth => 1,
            GraphPass::Shadow => 2,
            GraphPass::VgResolve => 3,
        }
    }
}

/// Bind group layouts of the depth and shadow variants that match the renderer's own passes.
pub(crate) struct PassBindLayouts {
    /// Camera and model storage of the normal prepass (group 0).
    pub depth_scene: wgpu::BindGroupLayout,
    /// Shadow matrix uniform of the shadow atlas (group 0).
    pub shadow_scene: wgpu::BindGroupLayout,
    /// Model storage of the shadow atlas (group 1).
    pub shadow_model: wgpu::BindGroupLayout,
    /// Model storage and page info (dynamic offset) of the virtual shadow pages (group 0).
    pub vsm_scene: wgpu::BindGroupLayout,
}

impl PassBindLayouts {
    /// Creates the layouts. wgpu deduplicates layouts with equal entries, so bind groups the
    /// renderer creates from its own passes stay compatible.
    pub fn new(device: &wgpu::Device) -> Self {
        let depth_scene = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("matgraph_depth_scene_bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                storage_entry(1, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let shadow_scene = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("matgraph_shadow_scene_bgl"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let shadow_model = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("matgraph_shadow_model_bgl"),
            entries: &[storage_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let vsm_scene = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("matgraph_vsm_scene_bgl"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(256),
                    },
                    count: None,
                },
            ],
        });
        Self {
            depth_scene,
            shadow_scene,
            shadow_model,
            vsm_scene,
        }
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Module source of the depth variant: `normal.wgsl`'s outputs, with the graph parameters in
/// group 1 and an alpha cut-out.
pub fn depth_source(generated: &Generated) -> String {
    shader_with_vertex_decode(&format!(
        "{MATERIAL_SURFACE_WGSL}\n{}\n{}\n{DEPTH_TEMPLATE}",
        generated.uniform_wgsl, generated.wgsl
    ))
}

/// Module source of the shadow variant: depth only, graph parameters in group 2 and an alpha
/// cut-out.
pub fn shadow_source(generated: &Generated) -> String {
    shader_with_vertex_decode(&format!(
        "{MATERIAL_SURFACE_WGSL}\n{}\n{}\n{SHADOW_TEMPLATE}",
        generated.uniform_wgsl, generated.wgsl
    ))
}

/// Module source of the sun's shadow page variant: depth only, graph parameters in group 1,
/// alpha cut-out and the vertex offset.
pub fn vsm_source(generated: &Generated) -> String {
    shader_with_vertex_decode(&format!(
        "{MATERIAL_SURFACE_WGSL}\n{}\n{}\n{VSM_TEMPLATE}",
        generated.uniform_wgsl, generated.wgsl
    ))
}

/// Whether two objects may share one draw run in a prepass or shadow pass. Objects of graph
/// slots only share a run with the same material slot, because each slot has its own pipeline
/// and parameter block. Objects of other slots batch as before.
pub(crate) fn batch_compatible(graphs: Option<&MaterialGraphs>, a: usize, b: usize) -> bool {
    let Some(graphs) = graphs else {
        return true;
    };
    (!graphs.is_graph(a) && !graphs.is_graph(b)) || a == b
}

/// Depth variant pipeline: the normal prepass targets and depth state.
pub(crate) fn depth_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some("matgraph_depth_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_graph_depth"),
            buffers: &[Some(CompressedVertex::buffer_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_graph_depth"),
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: NORMAL_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: NORMAL_SURFACE_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
    })
}

/// Shadow variant pipeline: the shadow atlas depth state with a discarding fragment stage.
pub(crate) fn shadow_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some("matgraph_shadow_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_graph_shadow"),
            buffers: &[Some(CompressedVertex::buffer_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_graph_shadow"),
            targets: &[],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: 0,
                slope_scale: 0.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
    })
}

/// Transparent variant pipeline: the transparent pass state (depth tested, not written, back
/// faces culled) with the given blend, and the graph parameter group.
pub(crate) fn blended_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    sample_count: u32,
    blend: wgpu::BlendState,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            buffers: &[Some(CompressedVertex::buffer_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: multisample_state(sample_count),
        multiview_mask: None,
    })
}

/// Depth variant: opacity cut-out and the normal and surface outputs of `normal.wgsl`.
const DEPTH_TEMPLATE: &str = r#"
// Graph depth prepass (MAT-02 step 5): normals and linear depth as normal.wgsl.
struct GraphCamera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct GraphModel {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

@group(0) @binding(0) var<uniform> graph_camera: GraphCamera;
@group(0) @binding(1) var<storage, read> graph_models: array<GraphModel>;
@group(1) @binding(0) var<uniform> mat_graph_params: MaterialParams;
@group(1) @binding(1) var<uniform> mat_graph_time: vec4<f32>;

const GRAPH_NORMAL_FAR: f32 = 1000.0;
const GRAPH_ALPHA_CUTOFF: f32 = 0.5;

struct GraphDepthVsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) @interpolate(flat) surface: vec2<f32>,
    @location(3) uv: vec2<f32>,
};

struct GraphDepthFragOut {
    @location(0) normal_depth: vec4<f32>,
    @location(1) surface: vec4<f32>,
};

fn graph_surface_input(world_pos: vec3<f32>, world_normal: vec3<f32>) -> SurfaceInput {
    return SurfaceInput(world_pos, world_normal, vec3<f32>(1.0, 0.0, 0.0), vec4<f32>(1.0), vec3<f32>(0.0));
}

@vertex
fn vs_graph_depth(in: CompressedVertIn, @builtin(instance_index) instance_idx: u32) -> GraphDepthVsOut {
    var out: GraphDepthVsOut;
    let mdl = graph_models[instance_idx];
    let local_pos = decode_local_position(in.pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    var world_pos = mdl.model * vec4<f32>(local_pos, 1.0);
    let normal_w = normalize(mdl.normal_matrix * oct_decode(in.normal_oct));
    world_pos = world_pos + vec4<f32>(material_vertex_offset_graph(graph_surface_input(world_pos.xyz, normal_w), in.uv, mat_graph_time.x, mat_graph_params), 0.0);
    out.clip_pos = graph_camera.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.world_normal = normal_w;
    out.surface = vec2<f32>(mdl.bounds_min.w - 2.0 * step(1.5, mdl.bounds_min.w), mdl.bounds_extent.w);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_graph_depth(in: GraphDepthVsOut) -> GraphDepthFragOut {
    let surf = material_surface_graph(graph_surface_input(in.world_pos, in.world_normal), in.uv, mat_graph_time.x, mat_graph_params);
    if surf.opacity < GRAPH_ALPHA_CUTOFF {
        discard;
    }
    let clip = graph_camera.view_proj * vec4<f32>(in.world_pos, 1.0);
    let view_z = max(abs(clip.w), 1e-5);
    var out: GraphDepthFragOut;
    out.normal_depth = vec4<f32>(normalize(in.world_normal), clamp(view_z / GRAPH_NORMAL_FAR, 0.0, 1.0));
    out.surface = vec4<f32>(in.surface, 0.0, 0.0);
    return out;
}
"#;

/// Shadow variant: depth only, with the opacity cut-out.
const SHADOW_TEMPLATE: &str = r#"
// Graph shadow pass (MAT-02 step 5): depth only, with the graph opacity cut-out.
struct GraphShadowMatrix {
    view_proj: mat4x4<f32>,
};

struct GraphShadowModel {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

@group(0) @binding(0) var<uniform> graph_shadow: GraphShadowMatrix;
@group(1) @binding(0) var<storage, read> graph_shadow_models: array<GraphShadowModel>;
@group(2) @binding(0) var<uniform> mat_graph_params: MaterialParams;
@group(2) @binding(1) var<uniform> mat_graph_time: vec4<f32>;

const GRAPH_SHADOW_ALPHA_CUTOFF: f32 = 0.5;

struct GraphShadowVsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

fn graph_shadow_surface_input(world_pos: vec3<f32>, world_normal: vec3<f32>) -> SurfaceInput {
    return SurfaceInput(world_pos, world_normal, vec3<f32>(1.0, 0.0, 0.0), vec4<f32>(1.0), vec3<f32>(0.0));
}

@vertex
fn vs_graph_shadow(in: CompressedVertIn, @builtin(instance_index) instance_idx: u32) -> GraphShadowVsOut {
    var out: GraphShadowVsOut;
    let mdl = graph_shadow_models[instance_idx];
    let local_pos = decode_local_position(in.pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    var world_pos = mdl.model * vec4<f32>(local_pos, 1.0);
    let normal_w = normalize(mdl.normal_matrix * oct_decode(in.normal_oct));
    world_pos = world_pos + vec4<f32>(material_vertex_offset_graph(graph_shadow_surface_input(world_pos.xyz, normal_w), in.uv, mat_graph_time.x, mat_graph_params), 0.0);
    out.clip_pos = graph_shadow.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.world_normal = normal_w;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_graph_shadow(in: GraphShadowVsOut) {
    let surf = material_surface_graph(graph_shadow_surface_input(in.world_pos, in.world_normal), in.uv, mat_graph_time.x, mat_graph_params);
    if surf.opacity < GRAPH_SHADOW_ALPHA_CUTOFF {
        discard;
    }
}
"#;

/// Sun shadow page variant: depth in the page atlas, with the vertex offset and the opacity cut-out.
const VSM_TEMPLATE: &str = r#"
// Graph VSM page pass (MAT-07): linear depth as vsm_depth.wgsl, graph parameters in group 1.
struct GraphVsmModel {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

struct GraphPageInfo {
    view_proj: mat4x4<f32>,
    atlas_offset_x: f32,
    atlas_offset_y: f32,
    page_size_px: f32,
    atlas_size_px: f32,
};

@group(0) @binding(0) var<storage, read> graph_vsm_models: array<GraphVsmModel>;
@group(0) @binding(1) var<uniform> graph_page: GraphPageInfo;
@group(1) @binding(0) var<uniform> mat_graph_params: MaterialParams;
@group(1) @binding(1) var<uniform> mat_graph_time: vec4<f32>;

struct GraphVsmVsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) depth: f32,
    @location(1) world_pos: vec3<f32>,
    @location(2) world_normal: vec3<f32>,
    @location(3) uv: vec2<f32>,
};

fn graph_vsm_surface_input(world_pos: vec3<f32>, world_normal: vec3<f32>) -> SurfaceInput {
    return SurfaceInput(world_pos, world_normal, vec3<f32>(1.0, 0.0, 0.0), vec4<f32>(1.0), vec3<f32>(0.0));
}

@vertex
fn vs_graph_vsm(in: CompressedVertIn, @builtin(instance_index) instance_idx: u32) -> GraphVsmVsOut {
    var out: GraphVsmVsOut;
    let mdl = graph_vsm_models[instance_idx];
    let local_pos = decode_local_position(in.pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    var world_pos = mdl.model * vec4<f32>(local_pos, 1.0);
    let normal_w = normalize(mdl.normal_matrix * oct_decode(in.normal_oct));
    world_pos = world_pos + vec4<f32>(material_vertex_offset_graph(graph_vsm_surface_input(world_pos.xyz, normal_w), in.uv, mat_graph_time.x, mat_graph_params), 0.0);
    out.clip_pos = graph_page.view_proj * world_pos;
    out.depth = out.clip_pos.z / out.clip_pos.w;
    out.world_pos = world_pos.xyz;
    out.world_normal = normal_w;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_graph_vsm(in: GraphVsmVsOut) -> @location(0) f32 {
    let surf = material_surface_graph(graph_vsm_surface_input(in.world_pos, in.world_normal), in.uv, mat_graph_time.x, mat_graph_params);
    if surf.opacity < 0.5 {
        discard;
    }
    return in.depth;
}
"#;

/// VSM page variant pipeline: the page atlas colour target, both faces cast.
pub(crate) fn vsm_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some("matgraph_vsm_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_graph_vsm"),
            buffers: &[Some(CompressedVertex::buffer_layout())],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_graph_vsm"),
            targets: &[Some(wgpu::ColorTargetState {
                format: crate::shadow::shadow_atlas::VSM_ATLAS_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matgraph_cache::validate_wgsl;
    use redox_matgraph::{CodegenOptions, Features, MaterialGraph, Node, NodeKind, PortRef};

    fn tint_graph() -> MaterialGraph {
        MaterialGraph {
            version: 1,
            name: "passes_test".into(),
            nodes: vec![
                Node {
                    id: 1,
                    kind: NodeKind::ConstVec3([0.2, 0.4, 0.6]),
                    inputs: vec![],
                },
                Node {
                    id: 2,
                    kind: NodeKind::SurfaceOutput,
                    inputs: vec![
                        Some(PortRef { node: 1, port: 0 }),
                        None,
                        None,
                        None,
                        None,
                        None,
                    ],
                },
            ],
        }
    }

    fn generated(pass: Pass) -> Generated {
        let variant = redox_matgraph::compile_variant(
            &tint_graph(),
            &CodegenOptions {
                pass,
                features: Features::default(),
            },
        )
        .expect("the test graph is valid");
        variant.generated
    }

    #[test]
    fn the_pass_codegen_and_groups_follow_the_pass() {
        assert_eq!(GraphPass::Alpha.codegen(), Pass::Main);
        assert_eq!(GraphPass::Depth.codegen(), Pass::Depth);
        assert_eq!(GraphPass::Shadow.codegen(), Pass::Shadow);
        assert_eq!(GraphPass::Forward.params_group(), GRAPH_PARAMS_GROUP);
        assert_eq!(GraphPass::Depth.params_group(), 1);
        assert_eq!(GraphPass::Shadow.params_group(), 2);
        assert_eq!(GraphPass::ALL.len(), 7);
        assert_eq!(GraphPass::VsmDepth.params_group(), 1);
        assert_eq!(GraphPass::VgResolve.params_group(), 3);
        assert_eq!(GraphPass::VgResolve.codegen(), Pass::Main);
    }

    #[test]
    fn the_depth_variant_is_valid_wgsl() {
        let source = depth_source(&generated(Pass::Depth));
        validate_wgsl(&source).expect("depth variant passes naga");
        assert!(source.contains("fs_graph_depth"));
        assert!(source.contains("@group(1) @binding(0)"));
    }

    #[test]
    fn the_shadow_variant_is_valid_wgsl() {
        let source = shadow_source(&generated(Pass::Shadow));
        validate_wgsl(&source).expect("shadow variant passes naga");
        assert!(source.contains("fs_graph_shadow"));
        assert!(source.contains("@group(2) @binding(0)"));
    }

    #[test]
    fn without_a_graph_store_every_run_batches() {
        assert!(batch_compatible(None, 3, 7));
    }
}
