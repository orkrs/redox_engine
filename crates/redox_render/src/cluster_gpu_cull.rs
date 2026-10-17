//! GPU compute pass for clustered forward light assignment.

use bytemuck::{Pod, Zeroable};

use crate::clustering::cluster_config;
use crate::shader::{CLUSTER_CULL_LIGHTS_WGSL, create_shader_module};

/// Per-frame parameters for the cluster light cull compute shader.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ClusterCullParams {
    /// Camera position (xyz) in world space.
    pub cam_pos: [f32; 4],
    /// View-projection matrix.
    pub view_proj: [[f32; 4]; 4],
    /// Number of lights to assign to clusters.
    pub num_lights: u32,
    /// Maximum lights stored per cluster.
    pub max_lights_per_cluster: u32,
    /// Size of the shared light-index buffer.
    pub indices_capacity: u32,
    /// Padding to the uniform alignment.
    pub _pad: u32,
}

/// GPU pipeline that assigns point lights to screen-space clusters.
pub struct ClusterGpuCull {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    counter_buffer: wgpu::Buffer,
}

impl ClusterGpuCull {
    /// Creates the compute pipeline and its layouts.
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cluster_cull_bgl"),
            entries: &[
                uniform_entry(0),
                uniform_entry(1),
                storage_read_entry(2),
                storage_read_entry(3),
                storage_rw_entry(4),
                storage_rw_entry(5),
                storage_rw_entry(6),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cluster_cull_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let shader = create_shader_module(
            device,
            "cluster_cull_lights_shader",
            CLUSTER_CULL_LIGHTS_WGSL,
        );
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("cluster_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_cull_params"),
            size: std::mem::size_of::<ClusterCullParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let counter_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_cull_global_offset"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bgl,
            params_buffer,
            counter_buffer,
        }
    }

    /// Clears the global light-index counter and records the cull dispatch.
    pub fn record_dispatch(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        params: &ClusterCullParams,
        cluster_info_buffer: &wgpu::Buffer,
        bounds_buffer: &wgpu::Buffer,
        point_lights_buffer: &wgpu::Buffer,
        metadata_buffer: &wgpu::Buffer,
        light_indices_buffer: &wgpu::Buffer,
        total_clusters: u32,
    ) {
        // Atomic offset into the compacted per-cluster light index list.
        queue.write_buffer(&self.counter_buffer, 0, bytemuck::bytes_of(&0u32));
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cluster_cull_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cluster_info_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: bounds_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: point_lights_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: metadata_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: light_indices_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.counter_buffer.as_entire_binding(),
                },
            ],
        });

        let workgroups = total_clusters.div_ceil(64);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cluster_cull_lights"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(workgroups.max(1), 1, 1);
        }
    }
}

impl Default for ClusterCullParams {
    fn default() -> Self {
        Self {
            cam_pos: [0.0; 4],
            view_proj: [[0.0; 4]; 4],
            num_lights: 0,
            max_lights_per_cluster: cluster_config::MAX_LIGHTS_PER_CLUSTER,
            indices_capacity: 0,
            _pad: 0,
        }
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

fn storage_read_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_rw_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
