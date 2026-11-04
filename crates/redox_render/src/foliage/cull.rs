//! GPU culling of a foliage layer: frustum test, distance LOD with hysteresis, and one
//! `DrawIndexedIndirect` record per LOD. The CPU reference is [`super::layer::cull_reference`].

use bytemuck::{Pod, Zeroable};

use super::layer::{FoliageLayer, FoliageView, LodRef, MAX_LODS, NO_LOD};
use crate::shader::create_shader_module;

const FOLIAGE_CULL_WGSL: &str = include_str!("../../shaders/foliage_cull.wgsl");

/// Bytes of one instance in the GPU buffer (WGSL struct stride).
const INSTANCE_BYTES: u64 = 32;
/// Bytes of one `DrawIndexedIndirect` record.
const INDIRECT_BYTES: u64 = 20;
/// Workgroup size of the `main` entry point in `foliage_cull.wgsl`.
const WORKGROUP_SIZE: u32 = 64;

/// Uniform of the cull pass; layout mirrors `CullParams` in `foliage_cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CullParams {
    planes: [[f32; 4]; 6],
    camera_pos: [f32; 4],
    lod_max_distance: [f32; 4],
    lod_index_count: [u32; 4],
    lod_count: u32,
    instance_count: u32,
    capacity: u32,
    bound_radius: f32,
}

/// GPU buffers of one uploaded layer. `prev_lod` persists across frames for the hysteresis.
pub struct FoliageGpu {
    instances: wgpu::Buffer,
    prev_lod: wgpu::Buffer,
    lod_counts: wgpu::Buffer,
    visible: wgpu::Buffer,
    indirect: wgpu::Buffer,
    lods: Vec<LodRef>,
    bound_radius: f32,
    instance_count: u32,
    capacity: u32,
}

impl FoliageGpu {
    /// Uploads the instances of `layer` and allocates the per-LOD outputs.
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, layer: &FoliageLayer) -> Self {
        let instance_count = layer.instances.len() as u32;
        // Buffers are never zero-sized, so an empty layer still gets one slot.
        let capacity = instance_count.max(1);
        let slots = capacity as u64;

        let instances = storage_buffer(
            device,
            "foliage_instances",
            slots * INSTANCE_BYTES,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        if instance_count > 0 {
            queue.write_buffer(&instances, 0, bytemuck::cast_slice(&layer.instances));
        }

        let prev_lod = storage_buffer(
            device,
            "foliage_prev_lod",
            slots * 4,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let no_lod = vec![NO_LOD; capacity as usize];
        queue.write_buffer(&prev_lod, 0, bytemuck::cast_slice(&no_lod));

        let lod_counts = storage_buffer(
            device,
            "foliage_lod_counts",
            MAX_LODS as u64 * 4,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let visible = storage_buffer(
            device,
            "foliage_visible",
            MAX_LODS as u64 * slots * 4,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let indirect = storage_buffer(
            device,
            "foliage_indirect",
            MAX_LODS as u64 * INDIRECT_BYTES,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC,
        );

        Self {
            instances,
            prev_lod,
            lod_counts,
            visible,
            indirect,
            lods: layer.lods.clone(),
            bound_radius: layer.bound_radius,
            instance_count,
            capacity,
        }
    }

    /// Indirect draw records: `MAX_LODS` records of five `u32` each.
    pub fn indirect_buffer(&self) -> &wgpu::Buffer {
        &self.indirect
    }

    /// Visible instance indices; LOD `l` occupies `[l * capacity, l * capacity + count_l)`.
    pub fn visible_buffer(&self) -> &wgpu::Buffer {
        &self.visible
    }

    /// Number of LODs of the layer.
    pub fn lod_count(&self) -> usize {
        self.lods.len()
    }

    /// Slots per LOD in the visible buffer.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    fn params(&self, view: &FoliageView) -> CullParams {
        let mut lod_max_distance = [0.0; 4];
        let mut lod_index_count = [0; 4];
        for (i, lod) in self.lods.iter().enumerate() {
            lod_max_distance[i] = lod.max_distance;
            lod_index_count[i] = lod.index_count;
        }
        CullParams {
            planes: crate::virtual_geometry::Frustum::from_view_proj(&view.view_proj).planes,
            camera_pos: [
                view.camera_pos[0],
                view.camera_pos[1],
                view.camera_pos[2],
                0.0,
            ],
            lod_max_distance,
            lod_index_count,
            lod_count: self.lods.len() as u32,
            instance_count: self.instance_count,
            capacity: self.capacity,
            bound_radius: self.bound_radius,
        }
    }
}

/// Compute pipelines of the foliage cull (`main` and `finalize`).
pub struct FoliageCull {
    cull: wgpu::ComputePipeline,
    finalize: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
}

impl FoliageCull {
    /// Creates the pipelines and the uniform buffer.
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("foliage_cull_bgl"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, false),
                storage_entry(3, false),
                storage_entry(4, false),
                storage_entry(5, false),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("foliage_cull_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let shader = create_shader_module(device, "foliage_cull_shader", FOLIAGE_CULL_WGSL);
        let cull = compute_pipeline(device, &layout, &shader, "main", "foliage_cull");
        let finalize = compute_pipeline(device, &layout, &shader, "finalize", "foliage_finalize");

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("foliage_cull_params"),
            size: std::mem::size_of::<CullParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            cull,
            finalize,
            bgl,
            params_buffer,
        }
    }

    /// Records the cull of `gpu` for `view`: clears the counters, culls, and writes the indirect
    /// records. Frame state (`prev_lod`) is updated in place on the GPU.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        gpu: &FoliageGpu,
        view: &FoliageView,
    ) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&gpu.params(view)),
        );
        encoder.clear_buffer(&gpu.lod_counts, 0, None);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("foliage_cull_bg"),
            layout: &self.bgl,
            entries: &[
                bind(0, self.params_buffer.as_entire_binding()),
                bind(1, gpu.instances.as_entire_binding()),
                bind(2, gpu.prev_lod.as_entire_binding()),
                bind(3, gpu.lod_counts.as_entire_binding()),
                bind(4, gpu.visible.as_entire_binding()),
                bind(5, gpu.indirect.as_entire_binding()),
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("foliage_cull"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.cull);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(gpu.instance_count.div_ceil(WORKGROUP_SIZE).max(1), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("foliage_finalize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.finalize);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
}

fn storage_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

fn compute_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    entry: &str,
    label: &str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        cache: crate::pipeline_cache::get(),
        label: Some(label),
        layout: Some(layout),
        module: shader,
        entry_point: Some(entry),
        compilation_options: Default::default(),
    })
}

fn bind(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
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

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
