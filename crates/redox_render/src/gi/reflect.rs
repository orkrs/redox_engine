//! GPU part of the cache reflections (GI-04): a compute pass that traces one reflected ray per
//! pixel through the global SDF (`shaders/gi_reflect.wgsl`) into a `Rgba16Float` texture of
//! `(radiance, blend weight)`. The post chain blends that texture over the colour.

use bytemuck::{Pod, Zeroable};

use crate::camera::CameraUniform;
use crate::post::gi_reflect::GiReflectSettings;
use crate::sdf::global_gpu::GlobalSdfGpu;
use crate::shader::{GI_REFLECT_WGSL, create_shader_module};

/// Format of the reflection texture.
pub const REFLECT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Uniform of `struct ReflectParams` in `gi_reflect.wgsl` (160 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ReflectParamsGpu {
    inv_proj: [[f32; 4]; 4],
    inv_view: [[f32; 4]; 4],
    screen: [f32; 2],
    min_roughness: f32,
    max_roughness: f32,
    max_distance: f32,
    intensity: f32,
    frame: u32,
    _pad: u32,
}

/// Resources the pass reads.
pub struct ReflectInputs<'a> {
    /// Normal target (world normal xyz, view depth / 1000 in w).
    pub normal: &'a wgpu::TextureView,
    /// Material surface target (roughness r, metallic g).
    pub surface: &'a wgpu::TextureView,
    /// Global SDF the rays march through.
    pub sdf: &'a GlobalSdfGpu,
    /// Probe grid of the last update (`GiGridGpu`).
    pub grid: &'a wgpu::Buffer,
    /// Irradiance atlas (stable copy).
    pub irradiance: &'a wgpu::TextureView,
    /// Relocated probe offsets.
    pub probe_offset: &'a wgpu::Buffer,
}

/// Pipeline, parameters and the output texture of the pass.
pub struct GiReflect {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
    frame: u32,
    /// Frames the pass was recorded in (test hook).
    pub recorded_frames: u64,
}

fn texture_entry(binding: u32, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dim,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn make_texture(device: &wgpu::Device, size: [u32; 2]) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gi_reflect"),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: REFLECT_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

impl GiReflect {
    /// Creates the pipeline and an output texture of `size` pixels.
    pub fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi_reflect_bgl"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, wgpu::TextureViewDimension::D2),
                texture_entry(2, wgpu::TextureViewDimension::D2),
                texture_entry(3, wgpu::TextureViewDimension::D3),
                texture_entry(4, wgpu::TextureViewDimension::D3),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(6),
                uniform_entry(7),
                texture_entry(8, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: REFLECT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gi_reflect_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let module = create_shader_module(device, "gi_reflect", GI_REFLECT_WGSL);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gi_reflect"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_reflect_params"),
            size: std::mem::size_of::<ReflectParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gi_reflect_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (texture, view) = make_texture(device, size);
        Self {
            pipeline,
            bgl,
            params,
            sampler,
            texture,
            view,
            size,
            frame: 0,
            recorded_frames: 0,
        }
    }

    /// The `(radiance, weight)` texture the post chain reads.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Recreates the output texture for a new size; the caller rebuilds what binds `view()`.
    pub fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if size != self.size {
            let (texture, view) = make_texture(device, size);
            self.texture = texture;
            self.view = view;
            self.size = size;
        }
    }

    /// Records the pass for one frame.
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera: &CameraUniform,
        settings: &GiReflectSettings,
        inputs: &ReflectInputs<'_>,
    ) {
        let gpu = ReflectParamsGpu {
            inv_proj: camera.inv_proj,
            inv_view: camera.inv_view,
            screen: [self.size[0] as f32, self.size[1] as f32],
            min_roughness: settings.min_roughness,
            max_roughness: settings.max_roughness,
            max_distance: settings.max_distance,
            intensity: settings.intensity,
            frame: self.frame,
            _pad: 0,
        };
        self.frame = self.frame.wrapping_add(1);
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&gpu));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi_reflect_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(inputs.normal),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(inputs.surface),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&inputs.sdf.distance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&inputs.sdf.albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: inputs.sdf.info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: inputs.grid.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(inputs.irradiance),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: inputs.probe_offset.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&self.view),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gi_reflect"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(self.size[0].div_ceil(8), self.size[1].div_ceil(8), 1);
        self.recorded_frames += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_params_block_matches_the_wgsl_struct() {
        assert_eq!(std::mem::size_of::<ReflectParamsGpu>(), 160);
    }
}
