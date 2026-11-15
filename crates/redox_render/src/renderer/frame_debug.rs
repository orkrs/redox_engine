//! On-screen view of the frame targets, for a black final image.
//!
//! `REDOX_FRAME_DEBUG=split|chain|hdr|depth|normal|post` selects it at startup.
//! [`Renderer::cycle_frame_debug_view`] steps through the same modes.

use bytemuck::{Pod, Zeroable};

use super::Renderer;
use crate::shader::{FRAME_DEBUG_WGSL, create_shader_module};

/// Which frame target replaces the presented image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FrameDebugView {
    /// The normal tonemapped frame.
    #[default]
    Off,
    /// Four quadrants: HDR, depth, normals, post.
    Split,
    /// Four quadrants: drawn HDR, the image TAA/post reads, the post output, contrast depth.
    Chain,
    /// Resolved HDR before the post chain, exposure-tonemapped here.
    Hdr,
    /// Camera depth. White is the far clear; a flat color means the prepass wrote nothing.
    Depth,
    /// World normals from the prepass, mapped to 0..1.
    Normal,
    /// HDR after SSR, motion blur and fog, before bloom and the final tonemap.
    Post,
}

impl FrameDebugView {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("REDOX_FRAME_DEBUG") else {
            return Self::Off;
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "split" => Self::Split,
            "chain" => Self::Chain,
            "hdr" => Self::Hdr,
            "depth" => Self::Depth,
            "normal" | "normals" => Self::Normal,
            "post" => Self::Post,
            _ => Self::Off,
        }
    }

    fn as_u32(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Split => 1,
            Self::Hdr => 2,
            Self::Depth => 3,
            Self::Normal => 4,
            Self::Post => 5,
            Self::Chain => 6,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Off => Self::Split,
            Self::Split => Self::Chain,
            Self::Chain => Self::Hdr,
            Self::Hdr => Self::Depth,
            Self::Depth => Self::Normal,
            Self::Normal => Self::Post,
            Self::Post => Self::Off,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Split => "split (TL red=HDR, TR green=depth, BL blue=normals, BR yellow=post)",
            Self::Chain => {
                "chain (TL red=HDR drawn, TR green=what TAA/post reads, BL blue=post output, BR yellow=depth contrast)"
            }
            Self::Hdr => "hdr (scene before post)",
            Self::Depth => "depth (white=far/empty)",
            Self::Normal => "normals",
            Self::Post => "post (after SSR/fog/blur)",
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameDebugUniform {
    exposure: f32,
    mode: u32,
    _pad: [u32; 2],
}

struct FrameDebugGpu {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
}

impl FrameDebugGpu {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame_debug_bgl"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                tex_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
        let shader = create_shader_module(device, "frame_debug_shader", FRAME_DEBUG_WGSL);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("frame_debug_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("frame_debug_pipeline"),
            layout: Some(&pipeline_layout),
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
                    format,
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
        });
        let uniform = crate::resource::buffer::create_uniform_buffer(
            device,
            "frame_debug_uniform",
            bytemuck::bytes_of(&FrameDebugUniform {
                exposure: 1.0,
                mode: 1,
                _pad: [0; 2],
            }),
        );
        Self {
            format,
            pipeline,
            layout,
            uniform,
        }
    }

    fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface: &wgpu::TextureView,
        hdr: &wgpu::TextureView,
        post: &wgpu::TextureView,
        normal: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        chain: &wgpu::TextureView,
        exposure: f32,
        view: FrameDebugView,
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&FrameDebugUniform {
                exposure,
                mode: view.as_u32(),
                _pad: [0; 2],
            }),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame_debug_bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(post),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(normal),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(chain),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame_debug"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: surface,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(super) struct FrameDebugState {
    view: FrameDebugView,
    gpu: Option<FrameDebugGpu>,
    announced: bool,
    frames: u32,
    /// HDR and depth the TAA/post groups were last rebuilt against.
    groups_hdr: Option<wgpu::Texture>,
    groups_depth: Option<wgpu::Texture>,
}

impl FrameDebugState {
    pub(super) fn from_env() -> Self {
        Self {
            view: FrameDebugView::from_env(),
            gpu: None,
            announced: false,
            frames: 0,
            groups_hdr: None,
            groups_depth: None,
        }
    }

    /// Remember the textures TAA and post sample after the pool rebind.
    pub(super) fn note_bindings(&mut self, hdr: &wgpu::Texture, depth: &wgpu::Texture) {
        self.groups_hdr = Some(hdr.clone());
        self.groups_depth = Some(depth.clone());
    }
}

impl Renderer {
    /// The debug view currently drawn over the swapchain.
    pub fn frame_debug_view(&self) -> FrameDebugView {
        self.frame_debug.view
    }

    /// Whether the groups rebuilt after the last pool swap still name the textures drawn now.
    pub fn frame_binding_probe(&self) -> (&'static str, &'static str) {
        (
            binding_state(&self.frame_debug.groups_hdr, &self.hdr_texture),
            binding_state(&self.frame_debug.groups_depth, &self.depth_texture),
        )
    }

    /// Replaces the presented image with a frame target. `Off` restores the normal frame.
    pub fn set_frame_debug_view(&mut self, view: FrameDebugView) {
        if self.frame_debug.view != view {
            self.frame_debug.announced = false;
        }
        self.frame_debug.view = view;
    }

    /// Off → split → chain → hdr → depth → normals → post → off.
    pub fn cycle_frame_debug_view(&mut self) -> FrameDebugView {
        self.set_frame_debug_view(self.frame_debug.view.next());
        self.frame_debug.view
    }

    /// Draws the debug view on top of the tonemapped frame. `post` is the HDR image
    /// after the post chain (or the resolved scene when no effect ran).
    pub(super) fn paint_frame_debug(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        surface: &wgpu::TextureView,
        post: &wgpu::TextureView,
    ) {
        let view = self.frame_debug.view;
        if view == FrameDebugView::Off {
            return;
        }
        let format = self.config.format;
        if self
            .frame_debug
            .gpu
            .as_ref()
            .is_none_or(|gpu| gpu.format != format)
        {
            self.frame_debug.gpu = Some(FrameDebugGpu::new(&self.device, format));
        }
        self.log_frame_debug();
        let hdr = self.hdr_view.clone();
        let normal = self.normal_view.clone();
        let depth = self.depth_view.clone();
        let chain = if self.taa_enabled {
            let slot = self.taa_history_read_idx;
            self.taa_history_view[slot].clone()
        } else {
            hdr.clone()
        };
        let exposure = self.exposure;
        let gpu = self.frame_debug.gpu.as_ref().expect("frame debug pipeline");
        gpu.record(
            &self.device,
            &self.queue,
            encoder,
            surface,
            &hdr,
            post,
            &normal,
            &depth,
            &chain,
            exposure,
            view,
        );
    }

    fn log_frame_debug(&mut self) {
        let view = self.frame_debug.view;
        if !self.frame_debug.announced {
            log::info!(
                "frame debug: {label}. A colored square at the center means this overlay ran; a black region next to it means that target is empty.",
                label = view.label(),
            );
            self.frame_debug.announced = true;
            self.frame_debug.frames = 0;
        }
        self.frame_debug.frames = self.frame_debug.frames.wrapping_add(1);
        if self.frame_debug.frames % 120 != 1 {
            return;
        }
        let objects = self.main_vis_scratch.len();
        let visible = self.main_vis_scratch.iter().filter(|on| **on).count();
        let path = if self.taa_enabled {
            "taa"
        } else if self.aa_sharp {
            "sharp"
        } else {
            "standard"
        };
        log::info!(
            "frame debug stats: path={path} msaa={} visible={visible}/{objects} sky={} ibl={} post={} hdr_binding={} depth_binding={}",
            self.msaa_sample_count,
            self.skybox_pass.is_some(),
            !self.ibl_resource.is_dummy(),
            post_list(&self.post_fx_settings),
            binding_state(&self.frame_debug.groups_hdr, &self.hdr_texture),
            binding_state(&self.frame_debug.groups_depth, &self.depth_texture),
        );
    }
}

fn binding_state(bound: &Option<wgpu::Texture>, drawn: &wgpu::Texture) -> &'static str {
    match bound {
        Some(texture) if texture == drawn => "current",
        Some(_) => "stale",
        None => "unknown",
    }
}

fn post_list(settings: &crate::post::post_fx::PostFxSettings) -> String {
    let mut parts = Vec::new();
    if settings.sss.is_some() {
        parts.push("sss");
    }
    if settings.gi_reflect.is_some() {
        parts.push("gi_reflect");
    }
    if settings.ssr.is_some() {
        parts.push("ssr");
    }
    if settings.motion_blur.is_some() {
        parts.push("motion");
    }
    if settings.dof.is_some() {
        parts.push("dof");
    }
    if settings.fog.is_some() {
        parts.push(if settings.fog_froxel {
            "fog_froxel"
        } else {
            "fog"
        });
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join("+")
    }
}
