//! Visibility analysis — determines which virtual pages are needed for the
//! current frame by projecting screen-space tiles through the depth buffer
//! into each clipmap level's light-space.
//!
//! Implemented as a compute pass dispatched over 8×8 screen tiles.

use wgpu;

use crate::shader::manager::create_shader_module;

/// WGSL source for the visibility compute shader.
pub const VSM_VISIBILITY_SHADER_SRC: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct ClipmapLevel {
    view_proj: mat4x4<f32>,
    world_half_extent: f32,
    page_table_offset: u32,
    pages_per_side: u32,
    _pad: u32,
};

struct VsmVisInfo {
    inv_vp: mat4x4<f32>,
    num_clipmap_levels: u32,
    screen_width: u32,
    screen_height: u32,
    _pad: u32,
};

@group(0) @binding(0) var t_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> vis_info: VsmVisInfo;
@group(0) @binding(2) var<storage, read> clipmap_levels: array<ClipmapLevel>;
@group(0) @binding(3) var<storage, read_write> requested_pages: array<atomic<u32>>;

fn reconstruct_world_pos(uv: vec2<f32>, depth: f32, inv_vp: mat4x4<f32>) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world_h = inv_vp * ndc;
    return world_h.xyz / world_h.w;
}

@compute @workgroup_size(8, 8, 1)
fn cs_visibility(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= vis_info.screen_width || gid.y >= vis_info.screen_height) {
        return;
    }

    let depth = textureLoad(t_depth, vec2<i32>(gid.xy), 0);
    if (depth >= 0.9999) { return; }

    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(f32(vis_info.screen_width), f32(vis_info.screen_height));
    let world_pos = reconstruct_world_pos(uv, depth, vis_info.inv_vp);

    for (var level = 0u; level < vis_info.num_clipmap_levels; level++) {
        let cm = clipmap_levels[level];
        let light_clip = cm.view_proj * vec4<f32>(world_pos, 1.0);
        let ndc = light_clip.xyz / light_clip.w;
        let lsuv = ndc.xy * 0.5 + 0.5;

        if (lsuv.x < 0.0 || lsuv.x > 1.0 || lsuv.y < 0.0 || lsuv.y > 1.0) {
            continue;
        }

        let page_x = u32(lsuv.x * f32(cm.pages_per_side));
        let page_y = u32(lsuv.y * f32(cm.pages_per_side));
        let flat_idx = cm.page_table_offset + page_y * cm.pages_per_side + page_x;

        let word_idx = flat_idx / 32u;
        let bit_idx = flat_idx % 32u;
        atomicOr(&requested_pages[word_idx], 1u << bit_idx);
    }
}
"#;

/// Resources for the visibility compute pass.
pub struct VisibilityPass {
    pub pipeline: wgpu::ComputePipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

/// Uniform uploaded each frame for the visibility shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VsmVisInfo {
    pub inv_vp: [[f32; 4]; 4],
    pub num_clipmap_levels: u32,
    pub screen_width: u32,
    pub screen_height: u32,
    pub _pad: u32,
}

impl VisibilityPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vsm_visibility_bgl"),
                entries: &[
                    // 0: depth texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Depth,
                        },
                        count: None,
                    },
                    // 1: VsmVisInfo uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 2: clipmap levels (storage, read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 3: requested pages (storage, read_write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vsm_visibility_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = create_shader_module(device, "vsm_visibility_shader", VSM_VISIBILITY_SHADER_SRC);

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vsm_visibility_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "cs_visibility",
            compilation_options: Default::default(),
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}
