//! Shared bind-group layouts and depth helpers used across render passes.
//!
//! Replaces the legacy `ForwardPass` name — the forward shading pipeline is
//! unused; PBR owns the main draw path. This module only exposes BGL factories
//! and [`ModelUniform`] shared by shadow, PBR, and debug passes.
//!
//! ## PBR bind-group split (Phase 2.2) — wired
//!
//! | Group | Role            | Bindings                                                        |
//! |-------|-----------------|----------------------------------------------------------------|
//! | **0** | Frame / global  | Camera, shader debug, IBL textures, BRDF LUT, blue noise       |
//! | **1** | Pass / scene    | Lights, shadows, clusters, VSM, DDGI, SDF atlas                |
//! | **2** | Materials       | Material uniform + albedo/normal/MR textures                   |
//! | **3** | Instances       | Model storage buffer                                           |

use crate::resource::texture::Texture;

/// Depth texture format used by opaque passes.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Multisample state for HDR geometry passes (color-only MSAA; depth stays 1×).
pub fn multisample_state(count: u32) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: count.max(1),
        mask: !0,
        alpha_to_coverage_enabled: false,
    }
}

/// Shared bind-group layouts for camera, light, model, and texture bindings.
pub struct SharedLayouts {
    /// Camera uniform layout.
    pub camera_bgl: wgpu::BindGroupLayout,
    /// Light uniform layout.
    pub light_bgl: wgpu::BindGroupLayout,
    /// Per-object uniform layout.
    pub model_bgl: wgpu::BindGroupLayout,
    /// Texture and sampler layout (2D).
    pub texture_bgl: wgpu::BindGroupLayout,
    /// Texture and sampler layout for 2D arrays.
    pub texture_array_bgl: wgpu::BindGroupLayout,
    /// Texture and sampler layout for cube maps.
    pub texture_cube_bgl: wgpu::BindGroupLayout,
    /// Texture and sampler layout for cube map arrays.
    pub texture_cube_array_bgl: wgpu::BindGroupLayout,
}

fn texture_layout(
    device: &wgpu::Device,
    label: &str,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Per-instance model data in the storage buffer.
///
/// Layout matches WGSL `Model`: mat4 (64B) + mat3x3 (48B) + vec4 color +
/// vec4 bounds_min + vec4 bounds_extent = 160 bytes.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelUniform {
    /// Model matrix.
    pub model: [[f32; 4]; 4],
    /// Normal matrix, three padded rows.
    pub normal_matrix: [[f32; 4]; 3],
    /// Object color (rgba).
    pub color: [f32; 4],
    /// Minimum corner of the object bounds (xyz).
    pub bounds_min: [f32; 4],
    /// Size of the object bounds (xyz).
    pub bounds_extent: [f32; 4],
}

impl SharedLayouts {
    /// Creates the layouts.
    pub fn new(device: &wgpu::Device) -> Self {
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let light_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let model_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let texture_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        Self {
            camera_bgl,
            light_bgl,
            model_bgl,
            texture_bgl,
            texture_array_bgl: texture_layout(
                device,
                "texture_array_bgl",
                wgpu::TextureViewDimension::D2Array,
            ),
            texture_cube_bgl: texture_layout(
                device,
                "texture_cube_bgl",
                wgpu::TextureViewDimension::Cube,
            ),
            texture_cube_array_bgl: texture_layout(
                device,
                "texture_cube_array_bgl",
                wgpu::TextureViewDimension::CubeArray,
            ),
        }
    }

    /// Creates a camera bind group.
    pub fn create_camera_bind_group(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bg"),
            layout: &self.camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    /// Creates a light bind group.
    pub fn create_light_bind_group(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light_bg"),
            layout: &self.light_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    /// Creates a per-object bind group.
    pub fn create_model_bind_group(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("model_bg"),
            layout: &self.model_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    /// Layout that matches the shape of `texture` (2D, array, cube or cube array).
    pub fn texture_layout_for(&self, texture: &Texture) -> &wgpu::BindGroupLayout {
        use wgpu::TextureViewDimension as D;
        match texture.view_dimension {
            D::D2Array => &self.texture_array_bgl,
            D::Cube => &self.texture_cube_bgl,
            D::CubeArray => &self.texture_cube_array_bgl,
            _ => &self.texture_bgl,
        }
    }

    /// Creates a texture bind group.
    pub fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        texture: &Texture,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture_bg"),
            layout: self.texture_layout_for(texture),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&texture.sampler),
                },
            ],
        })
    }
}

/// Creates a depth texture matching the given surface dimensions.
pub fn create_depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Backward-compatible alias used during migration.
pub type ForwardPass = SharedLayouts;

/// Factory for the planned frame-level bind group (camera + shared samplers).
pub struct PbrFrameLayouts {
    /// Camera uniform layout.
    pub camera_bgl: wgpu::BindGroupLayout,
    /// Sampler layout.
    pub sampler_bgl: wgpu::BindGroupLayout,
}

impl PbrFrameLayouts {
    /// Creates the layouts.
    pub fn new(device: &wgpu::Device) -> Self {
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr_frame_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let sampler_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr_frame_samplers_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        Self {
            camera_bgl,
            sampler_bgl,
        }
    }
}

/// Factory for the planned scene/shadow bind group (clusters, VSM, SDF).
pub struct PbrSceneLayouts {
    /// Light uniform layout.
    pub light_bgl: wgpu::BindGroupLayout,
    /// Shadow map layout.
    pub shadow_bgl: wgpu::BindGroupLayout,
}

impl PbrSceneLayouts {
    /// Creates the layouts.
    pub fn new(device: &wgpu::Device) -> Self {
        let light_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr_scene_light_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pbr_scene_shadow_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            }],
        });
        Self {
            light_bgl,
            shadow_bgl,
        }
    }
}

/// Documents the four-group PBR layout (wired in `pbr.wgsl` / `PbrPass`).
pub struct PbrBindGroupPlan;
