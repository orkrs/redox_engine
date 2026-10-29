//! Post effects between TAA and bloom: screen-space reflections, motion blur, depth of field
//! and height fog.
//!
//! The chain is `TAA output -> [SSR] -> [motion blur] -> [depth of field] -> [fog] -> bloom ->
//! tone mapping`. It runs only in the TAA branch, because the velocity and depth it reads are
//! produced there. Effects write their own targets and never the TAA history, so they do not
//! accumulate across frames. With every effect off, `PostFx::run_chain` records nothing and the
//! frame is bit-identical to a build without this module.

use bytemuck::{Pod, Zeroable};

use crate::camera::CameraUniform;
use crate::post::fog::{FOG_WGSL, FogSettings, fog_uniform};
use crate::post::ssr::{SSR_WGSL, SsrSettings, ssr_uniform};
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
    /// Screen-space reflections, or `None` when off. Runs first in the chain.
    pub ssr: Option<SsrSettings>,
    /// Height fog, or `None` when off. Runs last in the chain.
    pub fog: Option<FogSettings>,
}

/// Per-frame camera and sun data the effects read.
pub(crate) struct ChainFrame<'a> {
    /// Camera uniform of the frame (view, projection and their inverses, camera position).
    pub camera: &'a CameraUniform,
    /// Camera near and far planes.
    pub near_far: [f32; 2],
    /// Output size in pixels.
    pub screen: [f32; 2],
    /// Direction the sun shines in (xyz), from `LightUniform::dir_direction`.
    pub sun_direction: [f32; 4],
    /// Sun colour times intensity (rgb), from `LightUniform::dir_color`.
    pub sun_color: [f32; 4],
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
/// Group 0 is `0` colour, then the extra textures (velocity, depth, normal, surface), then the
/// sampler and the uniform as the last two bindings.
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

    /// Bind group over `textures` (bindings `0..n`), then the sampler (`n`) and the uniform (`n + 1`).
    fn bind_group(
        &self,
        device: &wgpu::Device,
        textures: &[&wgpu::TextureView],
    ) -> wgpu::BindGroup {
        let mut entries: Vec<wgpu::BindGroupEntry<'_>> = textures
            .iter()
            .enumerate()
            .map(|(i, view)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(*view),
            })
            .collect();
        let sampler_binding = textures.len() as u32;
        entries.push(wgpu::BindGroupEntry {
            binding: sampler_binding,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: sampler_binding + 1,
            resource: self.uniform.as_entire_binding(),
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(self.label),
            layout: &self.layout,
            entries: &entries,
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
    /// Normal target (`Rgba16Float`: world normal, view depth / 1000); SSR reads it.
    pub normal_view: &'a wgpu::TextureView,
    /// Material surface target (`Rg8Unorm`: roughness, metallic); SSR reads it.
    pub surface_view: &'a wgpu::TextureView,
    /// Bloom mip 0, which the tone-mapping bind groups read.
    pub bloom_view: &'a wgpu::TextureView,
    /// Tone-mapping pipeline the bind groups are laid out against.
    pub tone_pipeline: &'a wgpu::RenderPipeline,
    /// Tone-mapping uniform buffer.
    pub tone_uniform: &'a wgpu::Buffer,
    /// Sampler shared with tone mapping.
    pub sampler: &'a wgpu::Sampler,
}

/// Passes of the chain. Each owns its pipeline, sampler and uniform buffer.
struct EffectPasses {
    ssr: EffectPass,
    motion_blur: EffectPass,
    dof: EffectPass,
    fog: EffectPass,
}

/// Bind groups of one effect. `history[s]` reads TAA history slot `s`; `target[i]` reads
/// effect target `i`. The chain picks the one that matches where its input currently is.
struct Lanes {
    history: [wgpu::BindGroup; 2],
    target: [wgpu::BindGroup; 2],
}

/// Bind groups of every effect.
struct EffectLanes {
    ssr: Lanes,
    motion_blur: Lanes,
    dof: Lanes,
    fog: Lanes,
}

/// Where the chain's current image lives: a TAA history slot, or effect target `i`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    History(usize),
    Target(usize),
}

impl Source {
    /// Target an effect reads `self` from and writes to: the first target when the input is
    /// the TAA history, otherwise the other one, so an effect never reads and writes one target.
    fn next_target(self) -> usize {
        match self {
            Source::History(_) => 0,
            Source::Target(i) => 1 - i,
        }
    }
}

impl Lanes {
    fn get(&self, source: Source) -> &wgpu::BindGroup {
        match source {
            Source::History(slot) => &self.history[slot],
            Source::Target(i) => &self.target[i],
        }
    }
}

/// Effect targets (ping-pong pair) and the passes that write them.
pub(crate) struct PostFx {
    _textures: [wgpu::Texture; 2],
    /// `views[i]` is target `i`; the chain's result is one of these.
    pub(crate) views: [wgpu::TextureView; 2],
    /// Tone-mapping bind groups, one per target.
    pub(crate) tone_bgs: [wgpu::BindGroup; 2],
    passes: EffectPasses,
    lanes: EffectLanes,
}

impl PostFx {
    /// Creates the effect targets and passes for a `width` x `height` output.
    pub(crate) fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        inputs: &PostFxInputs<'_>,
    ) -> Self {
        let passes = create_passes(device);
        let (textures, views) = create_targets(device, width, height);
        let (tone_bgs, lanes) = build_bindings(device, &passes, &views, inputs);
        Self {
            _textures: textures,
            views,
            tone_bgs,
            passes,
            lanes,
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
        let (tone_bgs, lanes) = build_bindings(device, &self.passes, &views, inputs);
        self._textures = textures;
        self.views = views;
        self.tone_bgs = tone_bgs;
        self.lanes = lanes;
    }

    /// Records the enabled effects after TAA, in the order SSR, motion blur, DoF, fog.
    ///
    /// `history_slot` is the TAA slot this frame wrote. Returns the index of the target that
    /// holds the result, or `None` when no effect is on, in which case nothing is recorded.
    pub(crate) fn run_chain(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        history_slot: usize,
        frame: &ChainFrame<'_>,
        settings: &PostFxSettings,
    ) -> Option<usize> {
        if settings.ssr.is_none()
            && settings.motion_blur.is_none()
            && settings.dof.is_none()
            && settings.fog.is_none()
        {
            return None;
        }
        let mut source = Source::History(history_slot);
        if let Some(s) = settings.ssr {
            let dst = source.next_target();
            self.passes.ssr.write(queue, &ssr_uniform(&s, frame));
            self.passes
                .ssr
                .record(encoder, self.lanes.ssr.get(source), &self.views[dst]);
            source = Source::Target(dst);
        }
        if let Some(mb) = settings.motion_blur {
            let dst = source.next_target();
            self.passes
                .motion_blur
                .write(queue, &motion_blur_uniform(&mb, frame.screen));
            self.passes.motion_blur.record(
                encoder,
                self.lanes.motion_blur.get(source),
                &self.views[dst],
            );
            source = Source::Target(dst);
        }
        if let Some(dof) = settings.dof {
            let dst = source.next_target();
            self.passes
                .dof
                .write(queue, &dof_uniform(&dof, frame.screen, frame.near_far));
            self.passes
                .dof
                .record(encoder, self.lanes.dof.get(source), &self.views[dst]);
            source = Source::Target(dst);
        }
        if let Some(fog) = settings.fog {
            let dst = source.next_target();
            self.passes.fog.write(queue, &fog_uniform(&fog, frame));
            self.passes
                .fog
                .record(encoder, self.lanes.fog.get(source), &self.views[dst]);
            source = Source::Target(dst);
        }
        match source {
            Source::Target(i) => Some(i),
            Source::History(_) => None,
        }
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

/// Builds the four effect passes (pipelines, samplers, uniform buffers).
fn create_passes(device: &wgpu::Device) -> EffectPasses {
    let ssr = EffectPass::new(
        device,
        "screen_space_reflections",
        SSR_WGSL,
        &[
            texture_entry(0, FILTERABLE),
            texture_entry(1, FILTERABLE),
            texture_entry(2, FILTERABLE),
            sampler_entry(3),
            uniform_entry(4),
        ],
        std::mem::size_of::<crate::post::ssr::SsrUniform>() as u64,
    );
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
    let fog = EffectPass::new(
        device,
        "fog",
        FOG_WGSL,
        &[
            texture_entry(0, FILTERABLE),
            texture_entry(1, wgpu::TextureSampleType::Depth),
            sampler_entry(2),
            uniform_entry(3),
        ],
        std::mem::size_of::<crate::post::fog::FogUniform>() as u64,
    );
    EffectPasses {
        ssr,
        motion_blur,
        dof,
        fog,
    }
}

/// Bind groups of one effect: the colour input is the TAA history slot or effect target `i`,
/// the extra textures are the same for both.
fn lane_bindings(
    device: &wgpu::Device,
    pass: &EffectPass,
    history: [&wgpu::TextureView; 2],
    targets: &[wgpu::TextureView; 2],
    extra: &[&wgpu::TextureView],
) -> Lanes {
    let bind = |color: &wgpu::TextureView| {
        let mut textures = Vec::with_capacity(1 + extra.len());
        textures.push(color);
        textures.extend_from_slice(extra);
        pass.bind_group(device, &textures)
    };
    Lanes {
        history: [bind(history[0]), bind(history[1])],
        target: [bind(&targets[0]), bind(&targets[1])],
    }
}

/// Builds the tone-mapping bind groups over each target and the lanes of every effect.
fn build_bindings(
    device: &wgpu::Device,
    passes: &EffectPasses,
    views: &[wgpu::TextureView; 2],
    inputs: &PostFxInputs<'_>,
) -> ([wgpu::BindGroup; 2], EffectLanes) {
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
    let history = inputs.history_views;
    let lanes = EffectLanes {
        ssr: lane_bindings(
            device,
            &passes.ssr,
            history,
            views,
            &[inputs.normal_view, inputs.surface_view],
        ),
        motion_blur: lane_bindings(
            device,
            &passes.motion_blur,
            history,
            views,
            &[inputs.velocity_view],
        ),
        dof: lane_bindings(device, &passes.dof, history, views, &[inputs.depth_view]),
        fog: lane_bindings(device, &passes.fog, history, views, &[inputs.depth_view]),
    };
    ([tone(&views[0]), tone(&views[1])], lanes)
}

#[cfg(test)]
#[path = "post_fx_tests.rs"]
mod tests;
