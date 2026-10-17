//! SMAA 1× anti-aliasing pass (Subpixel Morphological Anti-Aliasing).
//!
//! Three-pass fullscreen post-process ported from iryoku's SMAA with
//! PRESET_HIGH configuration:
//! - threshold = 0.05
//! - max_search_steps = 16
//! - max_search_steps_diag = 8
//! - corner_rounding = 25
//!
//! ## Pipeline
//! 1. **Edge detection** (luma): LDR input → `edges_tex` (Rg8Unorm)
//! 2. **Blend weight calculation**: edges + area/search LUTs → `blend_tex` (Rgba8Unorm)
//! 3. **Neighborhood blending**: LDR input + blend weights → anti-aliased output

use crate::shader::{
    SMAA_BLEND_WGSL, SMAA_EDGE_WGSL, SMAA_NEIGHBORHOOD_WGSL, create_shader_module,
};
use bytemuck::{Pod, Zeroable};

// ─── Precomputed lookup textures (embedded at compile time) ──────────────────

const AREA_TEX_BYTES: &[u8] = include_bytes!("../../assets/smaa/AreaTexDX10.bin");
const SEARCH_TEX_BYTES: &[u8] = include_bytes!("../../assets/smaa/SearchTex.bin");

const AREA_TEX_WIDTH: u32 = 160;
const AREA_TEX_HEIGHT: u32 = 560;
const SEARCH_TEX_WIDTH: u32 = 64;
const SEARCH_TEX_HEIGHT: u32 = 16;

// ─── Uniform ─────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SmaaMetrics {
    rt_metrics: [f32; 4], // (1/w, 1/h, w, h)
}

// ─── Pass struct ─────────────────────────────────────────────────────────────

/// Subpixel morphological anti-aliasing: edge detection, blend weights, neighborhood blending.
pub struct SmaaPass {
    edge_pipeline: wgpu::RenderPipeline,
    edge_bgl: wgpu::BindGroupLayout,
    blend_pipeline: wgpu::RenderPipeline,
    blend_bgl: wgpu::BindGroupLayout,
    neighborhood_pipeline: wgpu::RenderPipeline,
    neighborhood_bgl: wgpu::BindGroupLayout,

    sampler_linear: wgpu::Sampler,
    sampler_point: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,

    area_tex_view: wgpu::TextureView,
    search_tex_view: wgpu::TextureView,
    _area_tex: wgpu::Texture,
    _search_tex: wgpu::Texture,

    edges_tex: wgpu::Texture,
    edges_view: wgpu::TextureView,
    blend_tex: wgpu::Texture,
    blend_view: wgpu::TextureView,

    width: u32,
    height: u32,
}

impl SmaaPass {
    /// Creates the pass and uploads its lookup textures.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        // ── Upload area + search textures ────────────────────────────────────
        let (area_tex, area_tex_view) = upload_lut(
            device,
            queue,
            "smaa_area_tex",
            AREA_TEX_WIDTH,
            AREA_TEX_HEIGHT,
            wgpu::TextureFormat::Rg8Unorm,
            AREA_TEX_BYTES,
            2,
        );
        let (search_tex, search_tex_view) = upload_lut(
            device,
            queue,
            "smaa_search_tex",
            SEARCH_TEX_WIDTH,
            SEARCH_TEX_HEIGHT,
            wgpu::TextureFormat::R8Unorm,
            SEARCH_TEX_BYTES,
            1,
        );

        // ── Samplers ─────────────────────────────────────────────────────────
        let sampler_linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("smaa_linear_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sampler_point = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("smaa_point_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // ── Uniform buffer ───────────────────────────────────────────────────
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smaa_uniform"),
            size: std::mem::size_of::<SmaaMetrics>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Pass 1: Edge detection ───────────────────────────────────────────
        let edge_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smaa_edge_bgl"),
            entries: &[
                tex_entry(0, true),     // t_color (filterable)
                sampler_entry(1, true), // s_linear
                uniform_entry(2),       // uniform
            ],
        });
        let edge_pipeline = create_fullscreen_pipeline(
            device,
            "smaa_edge",
            SMAA_EDGE_WGSL,
            &edge_bgl,
            wgpu::TextureFormat::Rg8Unorm,
        );

        // ── Pass 2: Blend weight calculation ─────────────────────────────────
        let blend_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smaa_blend_bgl"),
            entries: &[
                tex_entry(0, true),      // t_edges
                tex_entry(1, true),      // t_area
                tex_entry(2, true),      // t_search
                sampler_entry(3, true),  // s_linear
                sampler_entry(4, false), // s_point
                uniform_entry(5),        // uniform
            ],
        });
        let blend_pipeline = create_fullscreen_pipeline(
            device,
            "smaa_blend",
            SMAA_BLEND_WGSL,
            &blend_bgl,
            wgpu::TextureFormat::Rgba8Unorm,
        );

        // ── Pass 3: Neighborhood blending ────────────────────────────────────
        let neighborhood_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smaa_neighborhood_bgl"),
            entries: &[
                tex_entry(0, true),      // t_color
                tex_entry(1, true),      // t_blend
                sampler_entry(2, true),  // s_linear
                sampler_entry(3, false), // s_point
                uniform_entry(4),        // uniform
            ],
        });
        let neighborhood_pipeline = create_fullscreen_pipeline(
            device,
            "smaa_neighborhood",
            SMAA_NEIGHBORHOOD_WGSL,
            &neighborhood_bgl,
            wgpu::TextureFormat::Rgba8Unorm,
        );

        // ── Intermediate textures (1×1 placeholder, resized on first use) ───
        let (edges_tex, edges_view) =
            create_intermediate(device, "smaa_edges", 1, 1, wgpu::TextureFormat::Rg8Unorm);
        let (blend_tex, blend_view) =
            create_intermediate(device, "smaa_blend", 1, 1, wgpu::TextureFormat::Rgba8Unorm);

        Self {
            edge_pipeline,
            edge_bgl,
            blend_pipeline,
            blend_bgl,
            neighborhood_pipeline,
            neighborhood_bgl,
            sampler_linear,
            sampler_point,
            uniform_buffer,
            area_tex_view,
            search_tex_view,
            _area_tex: area_tex,
            _search_tex: search_tex,
            edges_tex,
            edges_view,
            blend_tex,
            blend_view,
            width: 1,
            height: 1,
        }
    }

    /// Recreate intermediate textures and update the uniform with new dimensions.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;

        let (edges_tex, edges_view) = create_intermediate(
            device,
            "smaa_edges",
            width,
            height,
            wgpu::TextureFormat::Rg8Unorm,
        );
        self.edges_tex = edges_tex;
        self.edges_view = edges_view;

        let (blend_tex, blend_view) = create_intermediate(
            device,
            "smaa_blend",
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        self.blend_tex = blend_tex;
        self.blend_view = blend_view;
    }

    /// Record the full 3-pass SMAA pipeline.
    ///
    /// - `color_view`: LDR input texture (post-ACES, Rgba8Unorm).
    /// - `out_view`: destination for the anti-aliased result (same format).
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        out_view: &wgpu::TextureView,
    ) {
        // Upload uniform
        let metrics = SmaaMetrics {
            rt_metrics: [
                1.0 / self.width as f32,
                1.0 / self.height as f32,
                self.width as f32,
                self.height as f32,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&metrics));

        // ── Pass 1: Edge detection ───────────────────────────────────────────
        let edge_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smaa_edge_bg"),
            layout: &self.edge_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smaa_edge_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &self.edges_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.edge_pipeline);
            pass.set_bind_group(0, &edge_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // ── Pass 2: Blend weight calculation ─────────────────────────────────
        let blend_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smaa_blend_bg"),
            layout: &self.blend_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.edges_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.area_tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.search_tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_point),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smaa_blend_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: &self.blend_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.blend_pipeline);
            pass.set_bind_group(0, &blend_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // ── Pass 3: Neighborhood blending ────────────────────────────────────
        let neighborhood_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smaa_neighborhood_bg"),
            layout: &self.neighborhood_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.blend_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_point),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smaa_neighborhood_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: out_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.neighborhood_pipeline);
            pass.set_bind_group(0, &neighborhood_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn upload_lut(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    data: &[u8],
    bytes_per_pixel: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_pixel * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_intermediate(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
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
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader_source: &str,
    bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = create_shader_module(device, &format!("{label}_shader"), shader_source);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{label}_layout")),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
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
        multiview_mask: None,
    })
}

fn tex_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32, filtering: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(if filtering {
            wgpu::SamplerBindingType::Filtering
        } else {
            wgpu::SamplerBindingType::NonFiltering
        }),
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
