//! Post effects between TAA and bloom: motion blur, then depth of field.
//!
//! The chain is `TAA output -> [motion blur] -> [depth of field] -> bloom -> tone mapping`.
//! It runs only in the TAA branch, because the velocity and depth it reads are produced there.
//! Effects write their own targets and never the TAA history, so blur does not accumulate
//! across frames. With every effect off, [`PostFx::run_chain`] records nothing and the frame
//! is bit-identical to a build without this module.

use bytemuck::{Pod, Zeroable};

use crate::post::tone_mapping::create_tonemap_bind_group;
use crate::shader::create_shader_module;

/// Fewest taps a blur can use; fewer than two cannot average along a direction.
pub const MIN_MOTION_BLUR_TAPS: u32 = 2;
/// Most taps the motion blur shader loops over.
pub const MAX_MOTION_BLUR_TAPS: u32 = 32;
/// Blurs shorter than this many pixels are skipped, so the pixel stays sharp.
const MIN_BLUR_PIXELS: f32 = 0.5;
/// Taps used by the depth-of-field disc blur.
const DOF_TAPS: u32 = 16;
/// Golden angle in radians, used by the disc sample spiral.
const GOLDEN_ANGLE: f32 = 2.399_963_1;

const MOTION_BLUR_WGSL: &str = include_str!("../../shaders/motion_blur.wgsl");
const DOF_WGSL: &str = include_str!("../../shaders/dof.wgsl");
/// Format of every effect target (same as the TAA history).
const FX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const FILTERABLE: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };

/// Parameters of the motion blur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlurSettings {
    /// Fraction of the frame time the virtual shutter stays open: 0.5 = 180 degree shutter.
    pub shutter: f32,
    /// Longest blur in pixels (the tap offsets are scaled down to it).
    pub max_pixels: f32,
    /// Number of taps along the blur direction, clamped to `2..=32`.
    pub taps: u32,
}

impl Default for MotionBlurSettings {
    fn default() -> Self {
        Self {
            shutter: 0.5,
            max_pixels: 24.0,
            taps: 8,
        }
    }
}

/// Clamps a tap count into the range the shader supports.
fn clamp_taps(taps: u32) -> u32 {
    taps.clamp(MIN_MOTION_BLUR_TAPS, MAX_MOTION_BLUR_TAPS)
}

/// UV offsets of the taps for a pixel with the given velocity (UV per frame).
///
/// The offsets are symmetric around zero and spread over `velocity * shutter`. The total
/// length is limited to `max_pixels` on a target of `screen` pixels. Returns an empty list when
/// the blur would be shorter than half a pixel, which leaves the pixel sharp.
/// The WGSL version in `shaders/motion_blur.wgsl` must stay in sync with this function.
pub fn motion_blur_offsets(
    velocity_uv: [f32; 2],
    screen: [f32; 2],
    s: &MotionBlurSettings,
) -> Vec<[f32; 2]> {
    let mut v = [velocity_uv[0] * s.shutter, velocity_uv[1] * s.shutter];
    let len_px = ((v[0] * screen[0]).powi(2) + (v[1] * screen[1]).powi(2)).sqrt();
    // NaN also leaves the pixel sharp.
    if len_px.is_nan() || len_px < MIN_BLUR_PIXELS {
        return Vec::new();
    }
    if len_px > s.max_pixels {
        let k = s.max_pixels / len_px;
        v = [v[0] * k, v[1] * k];
    }
    let taps = clamp_taps(s.taps);
    let last = (taps - 1) as f32;
    (0..taps)
        .map(|i| {
            let t = i as f32 / last - 0.5;
            [v[0] * t, v[1] * t]
        })
        .collect()
}

/// Parameters of the depth of field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DofSettings {
    /// Distance from the camera that is perfectly sharp, in world units.
    pub focus_distance: f32,
    /// Lens strength: circle of confusion in pixels per unit of relative defocus.
    pub aperture: f32,
    /// Largest blur radius in pixels (the cost is bounded by it).
    pub max_radius_pixels: f32,
}

impl Default for DofSettings {
    fn default() -> Self {
        Self {
            focus_distance: 10.0,
            aperture: 6.0,
            max_radius_pixels: 8.0,
        }
    }
}

/// Radius of the circle of confusion in pixels for a pixel at linear distance `distance`.
///
/// `coc = clamp(aperture * |distance - focus| / distance, 0, max_radius)`. A non-finite or
/// non-positive distance gives 0. The WGSL version in `shaders/dof.wgsl` must stay in sync.
pub fn coc_pixels(distance: f32, s: &DofSettings) -> f32 {
    if !(distance.is_finite() && distance > 0.0) {
        return 0.0;
    }
    let coc = s.aperture * (distance - s.focus_distance).abs() / distance;
    coc.min(s.max_radius_pixels).max(0.0)
}

/// Linear view-space distance of a depth-buffer value.
///
/// The camera uses the standard depth convention (`perspective`, near maps to 0 and far to 1,
/// depth test `LessEqual`), so `d = near * far / (far - z * (far - near))`.
pub fn linear_distance(depth: f32, near: f32, far: f32) -> f32 {
    near * far / (far - depth * (far - near))
}

/// Disc sample offsets in the unit disc for `n` taps (sunflower spiral).
///
/// The first tap is the centre. The rest spread by the golden angle, with radius growing as
/// the square root of the index so the samples cover the disc evenly.
pub fn disc_taps(n: u32) -> Vec<[f32; 2]> {
    let n = n.max(1);
    (0..n)
        .map(|i| {
            let r = if n == 1 {
                0.0
            } else {
                (i as f32 / (n - 1) as f32).sqrt()
            };
            let a = i as f32 * GOLDEN_ANGLE;
            [r * a.cos(), r * a.sin()]
        })
        .collect()
}

/// Which post effects are on. `None` means the effect is off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PostFxSettings {
    /// Motion blur, or `None` when off.
    pub motion_blur: Option<MotionBlurSettings>,
    /// Depth of field, or `None` when off.
    pub dof: Option<DofSettings>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MotionBlurUniform {
    screen: [f32; 2],
    shutter: f32,
    max_pixels: f32,
    taps: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct DofUniform {
    screen: [f32; 2],
    near: f32,
    far: f32,
    focus: f32,
    aperture: f32,
    max_radius: f32,
    taps: u32,
}

fn motion_blur_uniform(s: &MotionBlurSettings, screen: [f32; 2]) -> MotionBlurUniform {
    MotionBlurUniform {
        screen,
        shutter: s.shutter,
        max_pixels: s.max_pixels,
        taps: clamp_taps(s.taps),
        _pad: [0; 3],
    }
}

fn dof_uniform(s: &DofSettings, screen: [f32; 2], near_far: [f32; 2]) -> DofUniform {
    DofUniform {
        screen,
        near: near_far[0],
        far: near_far[1],
        focus: s.focus_distance,
        aperture: s.aperture,
        max_radius: s.max_radius_pixels,
        taps: DOF_TAPS,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// One fullscreen effect pass: pipeline, group-0 layout, linear clamp sampler and uniform.
///
/// Group 0 is always `0` colour, `1` second texture (velocity or depth), `2` sampler, `3` uniform.
struct EffectPass {
    label: &'static str,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

impl EffectPass {
    fn new(
        device: &wgpu::Device,
        label: &'static str,
        source: &str,
        entries: &[wgpu::BindGroupLayoutEntry],
        uniform_size: u64,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        });
        let shader = create_shader_module(device, label, source);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some(label),
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
                    format: FX_FORMAT,
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            label,
            pipeline,
            layout,
            sampler,
            uniform,
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        color: &wgpu::TextureView,
        second: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(self.label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(second),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        })
    }

    fn write<T: Pod>(&self, queue: &wgpu::Queue, value: &T) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(value));
    }

    fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(self.label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: target,
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
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Views the effect targets and their bind groups are built from.
pub(crate) struct PostFxInputs<'a> {
    /// The two TAA history slots. The chain reads the one this frame wrote.
    pub history_views: [&'a wgpu::TextureView; 2],
    /// Velocity texture (`Rgba16Float`, xy = UV offset per frame).
    pub velocity_view: &'a wgpu::TextureView,
    /// Scene depth (`Depth32Float`, internal resolution).
    pub depth_view: &'a wgpu::TextureView,
    /// Bloom mip 0, which the tone-mapping bind groups read.
    pub bloom_view: &'a wgpu::TextureView,
    /// Tone-mapping pipeline the bind groups are laid out against.
    pub tone_pipeline: &'a wgpu::RenderPipeline,
    /// Tone-mapping uniform buffer.
    pub tone_uniform: &'a wgpu::Buffer,
    /// Sampler shared with tone mapping.
    pub sampler: &'a wgpu::Sampler,
}

struct BindGroups {
    tone: [wgpu::BindGroup; 2],
    motion_blur: [wgpu::BindGroup; 2],
    /// DoF bind groups: index 0 and 1 read a TAA history slot, index 2 reads target 0.
    dof: [wgpu::BindGroup; 3],
}

/// Effect targets (ping-pong pair) and the passes that write them.
pub(crate) struct PostFx {
    _textures: [wgpu::Texture; 2],
    /// `views[i]` is target `i`; the chain's result is one of these.
    pub(crate) views: [wgpu::TextureView; 2],
    /// Tone-mapping bind groups, one per target.
    pub(crate) tone_bgs: [wgpu::BindGroup; 2],
    motion_blur: EffectPass,
    dof: EffectPass,
    motion_blur_bgs: [wgpu::BindGroup; 2],
    dof_bgs: [wgpu::BindGroup; 3],
}

impl PostFx {
    /// Creates the effect targets and passes for a `width` x `height` output.
    pub(crate) fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        inputs: &PostFxInputs<'_>,
    ) -> Self {
        let motion_blur = EffectPass::new(
            device,
            "motion_blur",
            MOTION_BLUR_WGSL,
            &[
                texture_entry(0, FILTERABLE),
                texture_entry(1, FILTERABLE),
                sampler_entry(2),
                uniform_entry(3),
            ],
            std::mem::size_of::<MotionBlurUniform>() as u64,
        );
        let dof = EffectPass::new(
            device,
            "depth_of_field",
            DOF_WGSL,
            &[
                texture_entry(0, FILTERABLE),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                sampler_entry(2),
                uniform_entry(3),
            ],
            std::mem::size_of::<DofUniform>() as u64,
        );
        let (textures, views) = create_targets(device, width, height);
        let bgs = build_bind_groups(device, &motion_blur, &dof, &views, inputs);
        Self {
            _textures: textures,
            views,
            tone_bgs: bgs.tone,
            motion_blur,
            dof,
            motion_blur_bgs: bgs.motion_blur,
            dof_bgs: bgs.dof,
        }
    }

    /// Recreates the targets and every bind group after a resize.
    pub(crate) fn rebuild(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        inputs: &PostFxInputs<'_>,
    ) {
        let (textures, views) = create_targets(device, width, height);
        let bgs = build_bind_groups(device, &self.motion_blur, &self.dof, &views, inputs);
        self._textures = textures;
        self.views = views;
        self.tone_bgs = bgs.tone;
        self.motion_blur_bgs = bgs.motion_blur;
        self.dof_bgs = bgs.dof;
    }

    /// Records the enabled effects after TAA.
    ///
    /// `history_slot` is the TAA slot this frame wrote, `screen` the output size in pixels and
    /// `near_far` the camera planes. Returns the index of the target that holds the result, or
    /// `None` when no effect is on, in which case nothing is recorded.
    pub(crate) fn run_chain(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        history_slot: usize,
        screen: [f32; 2],
        near_far: [f32; 2],
        settings: &PostFxSettings,
    ) -> Option<usize> {
        if settings.motion_blur.is_none() && settings.dof.is_none() {
            return None;
        }
        let mut out = 0;
        let mut dof_source = &self.dof_bgs[history_slot];
        if let Some(mb) = settings.motion_blur {
            self.motion_blur
                .write(queue, &motion_blur_uniform(&mb, screen));
            self.motion_blur
                .record(encoder, &self.motion_blur_bgs[history_slot], &self.views[0]);
            dof_source = &self.dof_bgs[2];
        }
        if let Some(dof) = settings.dof {
            out = usize::from(settings.motion_blur.is_some());
            self.dof.write(queue, &dof_uniform(&dof, screen, near_far));
            self.dof.record(encoder, dof_source, &self.views[out]);
        }
        Some(out)
    }
}

/// Two `Rgba16Float` targets at output resolution.
fn create_targets(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> ([wgpu::Texture; 2], [wgpu::TextureView; 2]) {
    let make = |label: &str| {
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
            format: FX_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    };
    let (t0, v0) = make("post_fx_target_0");
    let (t1, v1) = make("post_fx_target_1");
    ([t0, t1], [v0, v1])
}

fn build_bind_groups(
    device: &wgpu::Device,
    motion_blur: &EffectPass,
    dof: &EffectPass,
    views: &[wgpu::TextureView; 2],
    inputs: &PostFxInputs<'_>,
) -> BindGroups {
    let tone = |view: &wgpu::TextureView| {
        create_tonemap_bind_group(
            device,
            inputs.tone_pipeline,
            view,
            inputs.bloom_view,
            inputs.sampler,
            inputs.tone_uniform,
        )
    };
    BindGroups {
        tone: [tone(&views[0]), tone(&views[1])],
        motion_blur: [
            motion_blur.bind_group(device, inputs.history_views[0], inputs.velocity_view),
            motion_blur.bind_group(device, inputs.history_views[1], inputs.velocity_view),
        ],
        dof: [
            dof.bind_group(device, inputs.history_views[0], inputs.depth_view),
            dof.bind_group(device, inputs.history_views[1], inputs.depth_view),
            dof.bind_group(device, &views[0], inputs.depth_view),
        ],
    }
}
