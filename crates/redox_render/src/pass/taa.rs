//! Temporal Anti-Aliasing (TAA) render pass.
//!
//! Blends the current jittered HDR frame with an accumulated history buffer
//! to produce a stable, alias-reduced output.  Two `Rgba16Float` textures are
//! kept in a ping-pong arrangement — one is read as history, the other is
//! written as the new accumulated result.
//!
//! ## Algorithm overview
//! 1. Sample the current HDR frame at the current UV.
//! 2. Build a 3×3 neighbourhood colour-AABB for ghosting rejection.
//! 3. Fetch the motion vector from the velocity buffer, compute reprojected UV.
//! 4. Sample history at the reprojected UV (bilinear).
//! 5. Clamp history to the colour AABB.
//! 6. Lerp: `result = mix(clamped_history, current, alpha)` (alpha ≈ 0.1).
//! 7. On the first frame (or resize) set `reset = 1` to skip history blending.
//!
//! ## Bind group (group 0)
//! | binding | resource          | format / type       |
//! |---------|-------------------|---------------------|
//! | 0       | current HDR       | `texture_2d<f32>`   |
//! | 1       | velocity texture  | `texture_2d<f32>`   |
//! | 2       | history texture   | `texture_2d<f32>`   |
//! | 3       | linear sampler    | `sampler`           |
//! | 4       | `TaaUniform`      | uniform buffer      |

use bytemuck::{Pod, Zeroable};

use crate::resource::buffer::create_uniform_buffer;
use crate::shader::manager::{TAA_SHADER_SRC, create_shader_module};

// ── Halton low-discrepancy sequence ─────────────────────────────────────────

/// Evaluates the Halton sequence at `index` for the given `base`.
///
/// Returns a value in (0, 1].  Using bases 2 and 3 gives excellent 2-D
/// coverage for subpixel jitter (the same bases as Unreal Engine TAA).
pub fn halton(mut index: u64, base: u64) -> f32 {
    let mut f = 1.0f32;
    let mut r = 0.0f32;
    while index > 0 {
        f /= base as f32;
        r += f * (index % base) as f32;
        index /= base;
    }
    r
}

/// Computes the subpixel jitter for `frame_index` in **NDC units** such that
/// `proj.z_axis.x -= jitter.x` and `proj.z_axis.y -= jitter.y` produce a
/// constant screen-space shift of ±0.5 pixels, independent of depth.
///
/// The returned delta is already sign-adjusted for the column-2 insertion
/// (see `apply_jitter_to_projection`).
pub fn halton_jitter_ndc(frame_index: u64, width: u32, height: u32) -> [f32; 2] {
    // Halton values are in (0,1] → shift to [-0.5, 0.5] pixel range
    let px = halton(frame_index + 1, 2) - 0.5; // base-2
    let py = halton(frame_index + 1, 3) - 0.5; // base-3
    // Convert pixel offset to NDC offset
    let nx = px * 2.0 / width as f32;
    let ny = py * 2.0 / height as f32;
    [nx, ny]
}

/// Applies a subpixel jitter to a projection matrix (glam column-major).
///
/// Modifies `proj.z_axis.{x,y}` (column 2, rows 0 and 1) so that the
/// resulting NDC position is shifted by `(jitter_ndc[0], jitter_ndc[1])`.
///
/// Derivation (perspective_rh, clip_w = -view_z):
/// ```text
/// NDC_x = (f/aspect * view_x  +  z_axis.x * view_z) / (-view_z)
///       = NDC_x_orig  -  z_axis.x
/// ```
/// Therefore setting `z_axis.x = -jitter_ndc_x` shifts NDC_x by +jitter_ndc_x.
pub fn apply_jitter_to_projection(
    proj: redox_math::Mat4,
    jitter_ndc: [f32; 2],
) -> redox_math::Mat4 {
    let mut p = proj;
    // z_axis is the third column (index 2) of a glam Mat4
    p.z_axis.x = -jitter_ndc[0];
    p.z_axis.y = -jitter_ndc[1];
    p
}

// ── Uniform ──────────────────────────────────────────────────────────────────

/// Parameters uploaded to the TAA shader every frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TaaUniform {
    /// Output (present) resolution in pixels.
    pub output_size: [f32; 2],
    /// Input (internal render) resolution in pixels.
    pub input_size: [f32; 2],
    /// Fraction of the *current* frame blended in (typical: 0.05–0.10).
    /// Lower → more stable but slower to clear ghosting.
    pub blend_alpha: f32,
    /// Set to `1` on the first frame after creation/resize; `0` otherwise.
    /// When non-zero the shader skips history blending (avoids stale history).
    pub reset: u32,
}

impl Default for TaaUniform {
    fn default() -> Self {
        Self {
            output_size: [1920.0, 1080.0],
            input_size: [1920.0, 1080.0],
            blend_alpha: 0.1,
            reset: 1,
        }
    }
}

// ── Pass struct ───────────────────────────────────────────────────────────────

/// The TAA render pass.
///
/// Owns the pipeline, sampler, and the uniform buffer.
/// The **ping-pong history textures** are owned by [`RenderContext`] so they
/// can be shared with the tone-mapping pass.
pub struct TaaPass {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Bilinear clamp sampler used for all TAA texture samples.
    pub sampler: wgpu::Sampler,
    /// Uniform buffer (`TaaUniform`), updated each frame.
    pub uniform_buffer: wgpu::Buffer,
}

impl TaaPass {
    /// Creates the TAA pass.
    ///
    /// `hdr_format` should match the HDR and history texture format
    /// (`Rgba16Float` in standard RedOx setup).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat) -> Self {
        // ── Bind group layout ───────────────────────────────────────────────
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("taa_bgl"),
                entries: &[
                    // 0 : current HDR frame
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float {
                                filterable: true,
                            },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // 1 : velocity texture (motion vectors)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float {
                                filterable: true,
                            },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // 2 : history texture (previous TAA output)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float {
                                filterable: true,
                            },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // 3 : bilinear sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // 4 : TaaUniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // ── Shader & pipeline ───────────────────────────────────────────────
        let shader = create_shader_module(device, "taa_shader", TAA_SHADER_SRC);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("taa_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("taa_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // ── Sampler ─────────────────────────────────────────────────────────
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("taa_linear_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // ── Uniform buffer ──────────────────────────────────────────────────
        let uniform_buffer = create_uniform_buffer(
            device,
            "taa_uniform",
            bytemuck::bytes_of(&TaaUniform::default()),
        );

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            uniform_buffer,
        }
    }

    /// Creates the bind group for one TAA invocation.
    ///
    /// The caller chooses which of the two ping-pong textures is the history
    /// (`history_view`) and which is the write target (passed to the render
    /// pass as a color attachment).
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        current_hdr_view: &wgpu::TextureView,
        velocity_view: &wgpu::TextureView,
        history_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(current_hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(velocity_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(history_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }
}

// ── Helper: create one ping-pong history texture ──────────────────────────────

/// Creates one of the two ping-pong TAA history textures.
///
/// Format: `Rgba16Float`.  Usages: `RENDER_ATTACHMENT | TEXTURE_BINDING`.
pub fn create_history_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Creates the velocity (motion-vector) texture.
///
/// Format: `Rgba16Float` (xy = UV delta, zw unused).
/// Usages: `RENDER_ATTACHMENT | TEXTURE_BINDING`.
pub fn create_velocity_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("velocity_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
