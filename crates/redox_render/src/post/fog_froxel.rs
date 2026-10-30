//! Froxel fog volume (PST-06 step 3): in-scatter and extinction are injected into a view-aligned
//! 3D grid, integrated front to back, and sampled by the fog composite pass. Sun in-scatter is
//! shadowed by the VSM clipmap, so light shafts stop at occluders.
//!
//! The grid is `FROXEL_WIDTH` x `FROXEL_HEIGHT` x `FROXEL_SLICES`. Slice boundaries follow
//! [`slice_depth`] between the camera near plane and `min(far, FROXEL_MAX_DISTANCE)`. The WGSL
//! side lives in `shaders/fog_inject.wgsl`, `shaders/fog_integrate.wgsl` and
//! `shaders/fog_composite.wgsl`; the constants and the slice mapping there must match this file.
//!
//! The volume is not temporally filtered yet, and it is only used when `PostFxSettings::fog_froxel`
//! is on. The analytic fog of step 1 stays the default.

use crate::post::fog::FogUniform;
use crate::shader::create_shader_module;

/// Grid width in froxels (screen x).
pub const FROXEL_WIDTH: u32 = 160;
/// Grid height in froxels (screen y).
pub const FROXEL_HEIGHT: u32 = 90;
/// Grid depth in slices (view z, exponentially distributed).
pub const FROXEL_SLICES: u32 = 64;
/// Far end of the grid in metres; the fog beyond it is treated as the far slice.
pub const FROXEL_MAX_DISTANCE: f32 = 120.0;

const INJECT_WGSL: &str = include_str!("../../shaders/fog_inject.wgsl");
const INTEGRATE_WGSL: &str = include_str!("../../shaders/fog_integrate.wgsl");
/// Fragment shader that composites the integrated grid over the scene.
pub(crate) const FROXEL_COMPOSITE_WGSL: &str = include_str!("../../shaders/fog_composite.wgsl");

/// Format of both grids: rgb in-scatter and extinction (inject), or accumulated in-scatter and
/// transmittance (integrated). Same format as the effect targets.
const VOLUME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Workgroup edge of the inject pass (4 x 4 x 4 froxels).
const INJECT_TILE: u32 = 4;
/// Workgroup edge of the integrate pass (8 x 8 columns, one thread walks all slices).
const INTEGRATE_TILE: u32 = 8;

/// Fragment-visible, filterable 3D float texture: the integrated grid, sampled trilinearly.
pub(crate) fn volume_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D3,
            multisampled: false,
        },
        count: None,
    }
}

/// GPU state of the froxel volume: the injected grid, the integrated grid and the two compute passes.
pub(crate) struct FroxelVolume {
    _inject_texture: wgpu::Texture,
    _integrated_texture: wgpu::Texture,
    /// Integrated grid (rgb = accumulated in-scatter, a = transmittance); the composite reads it.
    pub(crate) integrated_view: wgpu::TextureView,
    /// Copy of the fog uniform the compute passes read.
    uniform: wgpu::Buffer,
    inject_pipeline: wgpu::ComputePipeline,
    inject_bind_group: wgpu::BindGroup,
    integrate_pipeline: wgpu::ComputePipeline,
    integrate_bind_group: wgpu::BindGroup,
}

impl FroxelVolume {
    /// Creates both grids and the compute passes. `vsm_info` and `vsm_pages` are the VSM buffers
    /// the shadow lookup reads; they live as long as the renderer.
    pub(crate) fn new(
        device: &wgpu::Device,
        vsm_info: &wgpu::Buffer,
        vsm_pages: &wgpu::TextureView,
    ) -> Self {
        let inject_texture = create_volume(device, "fog_froxel_inject");
        let integrated_texture = create_volume(device, "fog_froxel_integrated");
        let inject_view = inject_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let integrated_view = integrated_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog_froxel_uniform"),
            size: std::mem::size_of::<FogUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let inject_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog_froxel_inject_bgl"),
            entries: &[
                compute_entry(0, uniform_ty()),
                compute_entry(1, storage_volume_ty()),
                compute_entry(2, storage_buffer_ty()),
                compute_entry(3, texture_ty(wgpu::TextureViewDimension::D2Array)),
            ],
        });
        let inject_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog_froxel_inject_bg"),
            layout: &inject_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&inject_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: vsm_info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(vsm_pages),
                },
            ],
        });
        let inject_pipeline = compute_pipeline(
            device,
            "fog_froxel_inject",
            &inject_layout,
            INJECT_WGSL,
            "cs_inject",
        );

        let integrate_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fog_froxel_integrate_bgl"),
                entries: &[
                    compute_entry(0, uniform_ty()),
                    compute_entry(1, texture_ty(wgpu::TextureViewDimension::D3)),
                    compute_entry(2, storage_volume_ty()),
                ],
            });
        let integrate_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fog_froxel_integrate_bg"),
            layout: &integrate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&inject_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&integrated_view),
                },
            ],
        });
        let integrate_pipeline = compute_pipeline(
            device,
            "fog_froxel_integrate",
            &integrate_layout,
            INTEGRATE_WGSL,
            "cs_integrate",
        );

        Self {
            _inject_texture: inject_texture,
            _integrated_texture: integrated_texture,
            integrated_view,
            uniform,
            inject_pipeline,
            inject_bind_group,
            integrate_pipeline,
            integrate_bind_group,
        }
    }

    /// Records the inject and integrate passes for this frame. Writes `uniform` to the compute
    /// uniform buffer first, so the caller only needs to build it once per frame.
    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        uniform: &FogUniform,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniform));
        {
            let mut inject = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fog_froxel_inject"),
                timestamp_writes: None,
            });
            inject.set_pipeline(&self.inject_pipeline);
            inject.set_bind_group(0, &self.inject_bind_group, &[]);
            inject.dispatch_workgroups(
                FROXEL_WIDTH.div_ceil(INJECT_TILE),
                FROXEL_HEIGHT.div_ceil(INJECT_TILE),
                FROXEL_SLICES.div_ceil(INJECT_TILE),
            );
        }
        {
            let mut integrate = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fog_froxel_integrate"),
                timestamp_writes: None,
            });
            integrate.set_pipeline(&self.integrate_pipeline);
            integrate.set_bind_group(0, &self.integrate_bind_group, &[]);
            integrate.dispatch_workgroups(
                FROXEL_WIDTH.div_ceil(INTEGRATE_TILE),
                FROXEL_HEIGHT.div_ceil(INTEGRATE_TILE),
                1,
            );
        }
    }
}

/// One 3D grid. Written by a storage binding, then read by the next pass.
fn create_volume(device: &wgpu::Device, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: FROXEL_WIDTH,
            height: FROXEL_HEIGHT,
            depth_or_array_layers: FROXEL_SLICES,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: VOLUME_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn compute_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

fn uniform_ty() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

/// Read-only storage buffer (the VSM info block).
fn storage_buffer_ty() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

/// Float texture read with `textureLoad` only, so it is not filterable.
fn texture_ty(view_dimension: wgpu::TextureViewDimension) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: false },
        view_dimension,
        multisampled: false,
    }
}

fn storage_volume_ty() -> wgpu::BindingType {
    wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format: VOLUME_FORMAT,
        view_dimension: wgpu::TextureViewDimension::D3,
    }
}

fn compute_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    source: &str,
    entry_point: &str,
) -> wgpu::ComputePipeline {
    let shader = create_shader_module(device, label, source);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some(entry_point),
        compilation_options: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post::fog::slice_depth;

    #[test]
    fn froxel_slice_depths_are_increasing_and_hit_the_ends() {
        let near = 0.1;
        let far = FROXEL_MAX_DISTANCE;
        assert!((slice_depth(0, FROXEL_SLICES, near, far) - near).abs() < 1e-6);
        assert!((slice_depth(FROXEL_SLICES, FROXEL_SLICES, near, far) - far).abs() < 1e-3 * far);
        for i in 0..FROXEL_SLICES {
            let a = slice_depth(i, FROXEL_SLICES, near, far);
            let b = slice_depth(i + 1, FROXEL_SLICES, near, far);
            assert!(b > a, "slice {i}: {a} must be below {b}");
        }
    }
}
