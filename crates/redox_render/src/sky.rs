//! Procedural atmosphere (SKY-02), after Hillaire 2020, "A Scalable and Production Ready
//! Sky and Atmosphere Rendering Technique".
//!
//! Units: radii and heights in km, scattering per km, sun illuminance in lux, so the sky
//! radiance comes out in nits. World units are taken as metres: the camera altitude is
//! `world_y * 0.001` km. The planet centre sits at the origin of the atmosphere model, the
//! ground at world `y = 0`.
//!
//! Per frame (`SkyMode::Procedural`): the sky-view LUT is rebuilt for the camera altitude and
//! sun direction, and the sky pass samples it. The IBL cube is rebuilt from the sky only when
//! the sun has moved by at least one degree (see [`SkyAtmosphere::cube_needs_rebuild`]).
//! Transmittance and multiple-scattering LUTs are built once, in [`SkyAtmosphere::new`].

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

use crate::pass::forward::DEPTH_FORMAT;
use crate::pass::shared_layouts::multisample_state;
use crate::renderer::Renderer;
use crate::resource::ibl::{IBLResource, Skybox};
use crate::shader::create_shader_module;

const SKY_COMMON_WGSL: &str = include_str!("../shaders/sky_common.wgsl");
const SKY_LOOKUP_WGSL: &str = include_str!("../shaders/sky_lookup.wgsl");
const SKY_TRANSMITTANCE_WGSL: &str = include_str!("../shaders/sky_transmittance.wgsl");
const SKY_MULTISCATTER_WGSL: &str = include_str!("../shaders/sky_multiscatter.wgsl");
const SKY_VIEW_WGSL: &str = include_str!("../shaders/sky_view.wgsl");
const SKY_CUBE_WGSL: &str = include_str!("../shaders/sky_cube.wgsl");
const SKY_PASS_WGSL: &str = include_str!("../shaders/sky.wgsl");

/// Transmittance LUT size (width: sun-zenith parameter, height: radius).
pub const TRANSMITTANCE_SIZE: (u32, u32) = (256, 64);
/// Multiple-scattering LUT size (square).
pub const MULTISCATTER_SIZE: u32 = 32;
/// Sky-view LUT size (width: azimuth, height: zenith parameter).
pub const SKY_VIEW_SIZE: (u32, u32) = (192, 108);
/// Edge length of each face of the IBL cube built from the sky.
pub const SKY_CUBE_SIZE: u32 = 128;

const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Sun-angle change that triggers an IBL cube rebuild (1 degree, in radians).
const CUBE_REBUILD_ANGLE: f32 = std::f32::consts::PI / 180.0;
/// Angular radius of the sun disc used until [`SkyAtmosphere::set_sun_angle`] is called
/// (0.0087 rad, the `DirectionalLight::new` default).
const DEFAULT_SUN_ANGLE: f32 = 0.0087;
/// Steps of the CPU reference integration.
const CPU_REFERENCE_STEPS: u32 = 256;

/// Sky model selected by the renderer. `Hdri` (default) draws the environment cube as before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkyMode {
    /// Environment cube from the HDR map (unchanged behaviour).
    #[default]
    Hdri,
    /// Procedural atmosphere with a sun disc; the IBL follows the sky.
    Procedural,
}

/// Physical parameters of the atmosphere. Defaults are the Earth values of Hillaire 2020
/// (table 1). Scattering values are per km.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmosphereParams {
    /// Planet (ground) radius, km.
    pub planet_radius_km: f32,
    /// Height of the top of the atmosphere above the ground, km.
    pub atmosphere_height_km: f32,
    /// Rayleigh scattering per channel, 1/km.
    pub rayleigh_scattering: [f32; 3],
    /// Rayleigh density scale height, km.
    pub rayleigh_scale_height: f32,
    /// Mie scattering per channel, 1/km.
    pub mie_scattering: [f32; 3],
    /// Mie absorption per channel, 1/km.
    pub mie_absorption: [f32; 3],
    /// Mie density scale height, km.
    pub mie_scale_height: f32,
    /// Henyey-Greenstein / Cornette-Shanks anisotropy of Mie scattering.
    pub mie_g: f32,
    /// Ozone absorption per channel, 1/km (tent density profile, 10 to 40 km).
    pub ozone_absorption: [f32; 3],
    /// Lambertian ground albedo per channel.
    pub ground_albedo: [f32; 3],
}

impl Default for AtmosphereParams {
    fn default() -> Self {
        Self {
            planet_radius_km: 6360.0,
            atmosphere_height_km: 100.0,
            rayleigh_scattering: [5.802e-3, 13.558e-3, 33.1e-3],
            rayleigh_scale_height: 8.0,
            mie_scattering: [3.996e-3; 3],
            mie_absorption: [4.40e-3; 3],
            mie_scale_height: 1.2,
            mie_g: 0.8,
            ozone_absorption: [0.650e-3, 1.881e-3, 0.085e-3],
            ground_albedo: [0.3; 3],
        }
    }
}

/// Per-frame sky inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkyFrame {
    /// Unit direction towards the sun (world space, y up).
    pub sun_dir: [f32; 3],
    /// Sun illuminance, lux per channel (`DirectionalLight` colour times intensity).
    pub sun_irradiance: [f32; 3],
    /// Camera altitude above the ground, km.
    pub camera_altitude_km: f32,
}

/// Uniform shared by all sky shaders. Layout MUST match `struct Atmosphere` in
/// `shaders/sky_common.wgsl`. Size 144 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AtmosphereUniform {
    /// x ground radius, y top radius, z Rayleigh height, w Mie height (km).
    pub radii: [f32; 4],
    /// Rayleigh scattering (xyz).
    pub rayleigh: [f32; 4],
    /// Mie scattering (xyz), g (w).
    pub mie_scatter: [f32; 4],
    /// Mie absorption (xyz).
    pub mie_absorb: [f32; 4],
    /// Ozone absorption (xyz).
    pub ozone_absorb: [f32; 4],
    /// Ground albedo (xyz).
    pub ground_albedo: [f32; 4],
    /// Sun direction (xyz).
    pub sun_dir: [f32; 4],
    /// Sun illuminance (xyz), sun angular radius (w, rad).
    pub sun_irradiance: [f32; 4],
    /// Camera radius from the planet centre, km (x).
    pub view: [f32; 4],
}

impl AtmosphereUniform {
    /// Packs the parameters and the frame inputs.
    pub fn new(params: &AtmosphereParams, frame: &SkyFrame, sun_angle: f32) -> Self {
        let v3 = |a: [f32; 3], w: f32| [a[0], a[1], a[2], w];
        Self {
            radii: [
                params.planet_radius_km,
                params.planet_radius_km + params.atmosphere_height_km,
                params.rayleigh_scale_height,
                params.mie_scale_height,
            ],
            rayleigh: v3(params.rayleigh_scattering, 0.0),
            mie_scatter: v3(params.mie_scattering, params.mie_g),
            mie_absorb: v3(params.mie_absorption, 0.0),
            ozone_absorb: v3(params.ozone_absorption, 0.0),
            ground_albedo: v3(params.ground_albedo, 0.0),
            sun_dir: v3(frame.sun_dir, 0.0),
            sun_irradiance: v3(frame.sun_irradiance, sun_angle),
            view: [
                params.planet_radius_km + frame.camera_altitude_km,
                0.0,
                0.0,
                0.0,
            ],
        }
    }
}

/// Tent-shaped ozone density, 1 at 25 km, 0 at 10 km and 40 km. Same as `medium_at` in WGSL.
fn ozone_density(h: f32) -> f32 {
    let v = if h < 25.0 {
        h / 15.0 - 2.0 / 3.0
    } else {
        -h / 15.0 + 8.0 / 3.0
    };
    v.clamp(0.0, 1.0)
}

/// Extinction per channel (1/km) at radius `r` km from the planet centre.
pub fn extinction_at(params: &AtmosphereParams, r: f32) -> [f32; 3] {
    let h = r - params.planet_radius_km;
    let rho_r = (-h / params.rayleigh_scale_height).exp();
    let rho_m = (-h / params.mie_scale_height).exp();
    let rho_o = ozone_density(h);
    std::array::from_fn(|c| {
        params.rayleigh_scattering[c] * rho_r
            + (params.mie_scattering[c] + params.mie_absorption[c]) * rho_m
            + params.ozone_absorption[c] * rho_o
    })
}

/// CPU reference transmittance from `height_km` above the ground, along a ray with cosine
/// `cos_zenith` to the top of the atmosphere. Midpoint integration with 256 steps. Zero when
/// the ray hits the ground.
pub fn transmittance_reference(
    params: &AtmosphereParams,
    height_km: f32,
    cos_zenith: f32,
) -> [f32; 3] {
    let rg = params.planet_radius_km;
    let rt = rg + params.atmosphere_height_km;
    let r = rg + height_km;
    let mu = cos_zenith.clamp(-1.0, 1.0);
    if mu < 0.0 && r * r * (1.0 - mu * mu) < rg * rg {
        return [0.0; 3];
    }
    let dist = -r * mu + (rt * rt - r * r + r * r * mu * mu).max(0.0).sqrt();
    let dt = dist / CPU_REFERENCE_STEPS as f32;
    let mut optical = [0.0f32; 3];
    for i in 0..CPU_REFERENCE_STEPS {
        let t = (i as f32 + 0.5) * dt;
        let r_t = (r * r + 2.0 * r * mu * t + t * t).max(0.0).sqrt();
        let ext = extinction_at(params, r_t);
        for c in 0..3 {
            optical[c] += ext[c] * dt;
        }
    }
    std::array::from_fn(|c| (-optical[c]).exp())
}

/// Transmittance LUT coordinates to (radius km, cosine). Same mapping as
/// `transmittance_uv_to_rmu` in `shaders/sky_common.wgsl`.
pub fn transmittance_uv_to_rmu(params: &AtmosphereParams, uv: [f32; 2]) -> (f32, f32) {
    let rg = params.planet_radius_km;
    let rt = rg + params.atmosphere_height_km;
    let h = (rt * rt - rg * rg).sqrt();
    let rho = h * uv[1];
    let r = (rho * rho + rg * rg).sqrt();
    let d_min = rt - r;
    let d_max = rho + h;
    let d = d_min + uv[0] * (d_max - d_min);
    let mu = if d > 0.0 {
        (h * h - rho * rho - d * d) / (2.0 * r * d)
    } else {
        1.0
    };
    (r, mu.clamp(-1.0, 1.0))
}

fn wgsl(parts: &[&str]) -> String {
    parts.join("\n")
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= 1e-12 {
        return [0.0, 1.0, 0.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn lut_view(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: LUT_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn compute_pipeline(
    device: &wgpu::Device,
    label: &str,
    source: &str,
    entry: &str,
) -> wgpu::ComputePipeline {
    let module = create_shader_module(device, label, source);
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some(label),
        layout: None,
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
    })
}

/// The LUT textures, their views and the samplers shared by all sky shaders.
struct Luts {
    transmittance: wgpu::Texture,
    transmittance_view: wgpu::TextureView,
    multiscatter: wgpu::Texture,
    multiscatter_view: wgpu::TextureView,
    sky_view: wgpu::Texture,
    sky_view_view: wgpu::TextureView,
    /// Clamped sampler for the transmittance and multiple-scattering LUTs.
    lut_sampler: wgpu::Sampler,
    /// Sampler for the sky-view LUT: azimuth wraps, zenith clamps.
    sky_sampler: wgpu::Sampler,
}

/// GPU state of the procedural sky: LUTs, compute pipelines and the sky pass.
pub struct SkyAtmosphere {
    params: AtmosphereParams,
    /// Angular radius of the sun disc, radians. Set through `Renderer::set_sky_sun_angle`.
    pub sun_angle: f32,
    uniform: wgpu::Buffer,
    luts: Luts,
    sky_view_pipeline: wgpu::ComputePipeline,
    sky_cube_pipeline: wgpu::ComputePipeline,
    sky_view_bind_group: wgpu::BindGroup,
    pass_pipeline: wgpu::RenderPipeline,
    pass_bind_group: wgpu::BindGroup,
    cube_sun_dir: Option<[f32; 3]>,
    cube_builds: u32,
}

impl SkyAtmosphere {
    /// Builds the pipelines and bakes the transmittance and multiple-scattering LUTs.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        params: AtmosphereParams,
        camera_buffer: &wgpu::Buffer,
        hdr_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky_atmosphere_uniform"),
            size: std::mem::size_of::<AtmosphereUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &uniform,
            0,
            bytemuck::bytes_of(&AtmosphereUniform::new(
                &params,
                &SkyFrame::default(),
                DEFAULT_SUN_ANGLE,
            )),
        );

        let (transmittance, transmittance_view) =
            lut_view(device, "sky_transmittance_lut", TRANSMITTANCE_SIZE);
        let ms_size = (MULTISCATTER_SIZE, MULTISCATTER_SIZE);
        let (multiscatter, multiscatter_view) = lut_view(device, "sky_multiscatter_lut", ms_size);
        let (sky_view, sky_view_view) = lut_view(device, "sky_view_lut", SKY_VIEW_SIZE);
        let lut_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky_lut_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky_view_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let transmittance_pipeline = compute_pipeline(
            device,
            "sky_transmittance_pipeline",
            &wgsl(&[SKY_COMMON_WGSL, SKY_TRANSMITTANCE_WGSL]),
            "transmittance",
        );
        let multiscatter_pipeline = compute_pipeline(
            device,
            "sky_multiscatter_pipeline",
            &wgsl(&[SKY_COMMON_WGSL, SKY_LOOKUP_WGSL, SKY_MULTISCATTER_WGSL]),
            "multiscatter",
        );
        let sky_view_pipeline = compute_pipeline(
            device,
            "sky_view_pipeline",
            &wgsl(&[SKY_COMMON_WGSL, SKY_LOOKUP_WGSL, SKY_VIEW_WGSL]),
            "sky_view",
        );
        let sky_cube_pipeline = compute_pipeline(
            device,
            "sky_cube_pipeline",
            &wgsl(&[SKY_COMMON_WGSL, SKY_LOOKUP_WGSL, SKY_CUBE_WGSL]),
            "sky_cube",
        );

        // One-off bake of the LUTs that do not depend on the sun or the camera.
        let transmittance_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_transmittance_bg"),
            layout: &transmittance_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
            ],
        });
        let multiscatter_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_multiscatter_bg"),
            layout: &multiscatter_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&multiscatter_view),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sky_bake_encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky_transmittance_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&transmittance_pipeline);
            pass.set_bind_group(0, &transmittance_bg, &[]);
            pass.dispatch_workgroups(TRANSMITTANCE_SIZE.0 / 8, TRANSMITTANCE_SIZE.1 / 8, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky_multiscatter_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&multiscatter_pipeline);
            pass.set_bind_group(0, &multiscatter_bg, &[]);
            pass.dispatch_workgroups(MULTISCATTER_SIZE / 8, MULTISCATTER_SIZE / 8, 1);
        }
        queue.submit(Some(encoder.finish()));

        let sky_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_view_bg"),
            layout: &sky_view_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&transmittance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&multiscatter_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&sky_view_view),
                },
            ],
        });

        let luts = Luts {
            transmittance,
            transmittance_view,
            multiscatter,
            multiscatter_view,
            sky_view,
            sky_view_view,
            lut_sampler,
            sky_sampler,
        };
        let (pass_pipeline, pass_bind_group) = build_pass(
            device,
            camera_buffer,
            &uniform,
            &luts,
            hdr_format,
            sample_count,
        );
        Self {
            params,
            sun_angle: DEFAULT_SUN_ANGLE,
            uniform,
            luts,
            sky_view_pipeline,
            sky_cube_pipeline,
            sky_view_bind_group,
            pass_pipeline,
            pass_bind_group,
            cube_sun_dir: None,
            cube_builds: 0,
        }
    }

    /// Recreates the sky pass pipeline for a new HDR format or sample count (window resize).
    /// LUTs are kept.
    pub fn rebuild_pass(
        &mut self,
        device: &wgpu::Device,
        camera_buffer: &wgpu::Buffer,
        hdr_format: wgpu::TextureFormat,
        sample_count: u32,
    ) {
        let (pipeline, bind_group) = build_pass(
            device,
            camera_buffer,
            &self.uniform,
            &self.luts,
            hdr_format,
            sample_count,
        );
        self.pass_pipeline = pipeline;
        self.pass_bind_group = bind_group;
    }

    /// Writes the frame uniform and records the sky-view LUT update into `encoder`.
    /// Call once per frame, before the sky pass draws.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: SkyFrame,
    ) {
        let frame = SkyFrame {
            sun_dir: normalize3(frame.sun_dir),
            ..frame
        };
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&AtmosphereUniform::new(
                &self.params,
                &frame,
                self.sun_angle,
            )),
        );
        self.record_sky_view(encoder);
    }

    fn record_sky_view(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sky_view_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.sky_view_pipeline);
        pass.set_bind_group(0, &self.sky_view_bind_group, &[]);
        pass.dispatch_workgroups(SKY_VIEW_SIZE.0.div_ceil(8), SKY_VIEW_SIZE.1.div_ceil(8), 1);
    }

    /// True when the IBL cube has never been built or the sun moved by at least one degree
    /// since the last build.
    pub fn cube_needs_rebuild(&self, sun_dir: [f32; 3]) -> bool {
        let sun = normalize3(sun_dir);
        match self.cube_sun_dir {
            None => true,
            Some(prev) => {
                let cos = prev[0] * sun[0] + prev[1] * sun[1] + prev[2] * sun[2];
                cos < CUBE_REBUILD_ANGLE.cos()
            }
        }
    }

    /// Renders the sky into a new cube map (`SKY_CUBE_SIZE`, Rgba16Float) for the IBL
    /// convolutions. Records its own submission: sky-view LUT, then the cube.
    pub fn build_cube(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: SkyFrame,
    ) -> Skybox {
        let frame = SkyFrame {
            sun_dir: normalize3(frame.sun_dir),
            ..frame
        };
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&AtmosphereUniform::new(
                &self.params,
                &frame,
                self.sun_angle,
            )),
        );
        let cubemap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky_ibl_cubemap"),
            size: wgpu::Extent3d {
                width: SKY_CUBE_SIZE,
                height: SKY_CUBE_SIZE,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LUT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = cubemap.create_view(&wgpu::TextureViewDescriptor {
            label: Some("sky_ibl_cubemap_view"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let storage_view = cubemap.create_view(&wgpu::TextureViewDescriptor {
            label: Some("sky_ibl_cubemap_storage_view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let cube_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky_cube_bg"),
            layout: &self.sky_cube_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.luts.sky_view_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.luts.sky_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&storage_view),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sky_cube_encoder"),
        });
        self.record_sky_view(&mut encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky_cube_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.sky_cube_pipeline);
            pass.set_bind_group(0, &cube_bg, &[]);
            let groups = SKY_CUBE_SIZE / 8;
            pass.dispatch_workgroups(groups, groups, 6);
        }
        queue.submit(Some(encoder.finish()));

        self.cube_sun_dir = Some(frame.sun_dir);
        self.cube_builds += 1;
        Skybox { cubemap, view }
    }

    /// Number of IBL cube rebuilds since creation.
    pub fn cube_builds(&self) -> u32 {
        self.cube_builds
    }

    /// Transmittance LUT texture (Rgba16Float, `TRANSMITTANCE_SIZE`).
    pub fn transmittance_lut(&self) -> &wgpu::Texture {
        &self.luts.transmittance
    }

    /// Multiple-scattering LUT texture (Rgba16Float, `MULTISCATTER_SIZE` square).
    pub fn multiscatter_lut(&self) -> &wgpu::Texture {
        &self.luts.multiscatter
    }

    /// Sky-view LUT texture from the last `prepare` (Rgba16Float, `SKY_VIEW_SIZE`).
    pub fn sky_view_lut(&self) -> &wgpu::Texture {
        &self.luts.sky_view
    }

    /// Parameters the LUTs were built with.
    pub fn params(&self) -> &AtmosphereParams {
        &self.params
    }

    /// Records the sky pass draw (fullscreen triangle). Call inside the forward HDR pass
    /// after the depth clear, like [`crate::pass::skybox::SkyboxPass::draw`].
    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pass_pipeline);
        render_pass.set_bind_group(0, &self.pass_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// Builds the sky pass pipeline and its bind group (bindings as in `shaders/sky.wgsl`).
fn build_pass(
    device: &wgpu::Device,
    camera_buffer: &wgpu::Buffer,
    uniform: &wgpu::Buffer,
    luts: &Luts,
    hdr_format: wgpu::TextureFormat,
    sample_count: u32,
) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
    let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sky_pass_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture_entry(1),
            sampler_entry(2),
            texture_entry(3),
            texture_entry(4),
            sampler_entry(5),
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky_pass_pipeline_layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let source = wgsl(&[SKY_COMMON_WGSL, SKY_LOOKUP_WGSL, SKY_PASS_WGSL]);
    let shader = create_shader_module(device, "sky_pass_shader", &source);
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some("sky_pass_pipeline"),
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
                format: hdr_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        // Same depth state as the skybox: far plane, overdrawn by opaque geometry.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: multisample_state(sample_count),
        multiview_mask: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sky_pass_bg"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&luts.transmittance_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&luts.lut_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&luts.multiscatter_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&luts.sky_view_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&luts.sky_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: camera_buffer.as_entire_binding(),
            },
        ],
    });
    (pipeline, bind_group)
}

impl Renderer {
    /// Procedural sky, once per frame, before the forward pass records draws: updates the
    /// sky-view LUT and, when the sun moved by at least one degree, rebuilds the IBL cube from
    /// the sky and installs it with [`Renderer::set_ibl_resource`]. No-op unless the sky is
    /// procedural.
    pub(crate) fn prepare_procedural_sky(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        cam_eye: Vec3,
    ) {
        let frame = self.sky_frame(cam_eye);
        let Some(sb) = self.skybox_pass.as_mut() else {
            return;
        };
        let Some(sky) = sb.sky.as_mut() else {
            return;
        };
        sky.prepare(&self.queue, encoder, frame);
        if !sky.cube_needs_rebuild(frame.sun_dir) {
            return;
        }
        let skybox = sky.build_cube(&self.device, &self.queue, frame);
        let resource =
            IBLResource::from_skybox(&self.device, &self.queue, &self.ibl_processor, skybox);
        let previous = self.swap_ibl_resource(resource);
        // The first resource replaced is the one `set_sky_mode(Hdri)` brings back.
        if self.pre_sky_ibl.is_none() {
            self.pre_sky_ibl = Some(previous);
        }
    }

    /// Sky inputs of the frame: sun from the light uniform, camera altitude from the eye
    /// (world units taken as metres).
    fn sky_frame(&self, cam_eye: Vec3) -> SkyFrame {
        let l = &self.light_uniform;
        SkyFrame {
            sun_dir: [l.dir_direction[0], l.dir_direction[1], l.dir_direction[2]],
            sun_irradiance: [l.dir_color[0], l.dir_color[1], l.dir_color[2]],
            camera_altitude_km: cam_eye.y.max(0.0) * 0.001,
        }
    }
}
