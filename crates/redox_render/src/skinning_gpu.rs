//! GPU side of [`crate::SkinnedMeshBinding::update_gpu`]: the compute pipeline and the per-binding
//! buffers (static bind-pose vertices, skin matrices, quantisation parameters).

use half::f16;
use redox_anim::SkinnedGeometry;
use redox_math::{Aabb, Mat4};
use wgpu::util::DeviceExt;

use crate::mesh::compress::bounds_vectors;
use crate::shader::{SKIN_COMPUTE_WGSL, create_shader_module};

const WORKGROUP_SIZE: u32 = 64;
const MAX_WORKGROUPS: u32 = 65_535;
const MATRIX_BYTES: u64 = 64;

/// Bind-pose vertex as the shader reads it (`SkinVertex` in `skin_compute.wgsl`, 80 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuSkinVertex {
    position: [f32; 3],
    uv_packed: u32,
    normal: [f32; 3],
    pad: u32,
    joints: [u32; 4],
    weights: [f32; 4],
    tangent: [f32; 3],
    /// Non-zero when `tangent` holds a real tangent; zero keeps the shader's fixed tangent.
    has_tangent: u32,
}

const _: () = assert!(std::mem::size_of::<GpuSkinVertex>() == 80);
const _: () = assert!(std::mem::offset_of!(GpuSkinVertex, tangent) == 64);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuSkinParams {
    bounds_min: [f32; 4],
    bounds_extent: [f32; 4],
    vertex_count: u32,
    matrix_count: u32,
    pad: [u32; 2],
}

/// The skinning compute pipeline, shared by every binding of a renderer.
pub(crate) struct SkinPipeline {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
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

impl SkinPipeline {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = create_shader_module(device, "skin_compute", SKIN_COMPUTE_WGSL);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skin_compute_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skin_compute_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("skin_compute_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
        });
        Self { pipeline, layout }
    }
}

/// GPU resources of one skinned mesh.
pub(crate) struct GpuSkin {
    skin_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Number of skin matrices the geometry needs (highest influencing joint + 1).
    required_matrices: usize,
    workgroups: u32,
}

impl GpuSkin {
    /// Creates the static buffers. `None` when the geometry is too large for one dispatch.
    pub(crate) fn new(
        device: &wgpu::Device,
        pipeline: &SkinPipeline,
        geometry: &SkinnedGeometry,
        bounds: &Aabb,
        out_vertices: &wgpu::Buffer,
    ) -> Option<Self> {
        let count = geometry.vertex_count();
        let workgroups = (count as u32).div_ceil(WORKGROUP_SIZE);
        if count == 0 || workgroups > MAX_WORKGROUPS {
            return None;
        }
        let required_matrices = geometry
            .joints
            .iter()
            .zip(&geometry.weights)
            .flat_map(|(j, w)| j.iter().zip(w).filter(|(_, w)| **w > 0.0).map(|(j, _)| *j))
            .map(|j| j as usize + 1)
            .max()
            .unwrap_or(0);

        let has_tangents = geometry.has_tangents();
        let vertices: Vec<GpuSkinVertex> = (0..count)
            .map(|i| GpuSkinVertex {
                position: geometry.positions[i],
                uv_packed: u32::from(f16::from_f32(geometry.uvs[i][0]).to_bits())
                    | (u32::from(f16::from_f32(geometry.uvs[i][1]).to_bits()) << 16),
                normal: geometry.normals[i],
                pad: 0,
                joints: geometry.joints[i].map(u32::from),
                weights: geometry.weights[i],
                tangent: if has_tangents {
                    geometry.tangents[i]
                } else {
                    [0.0; 3]
                },
                has_tangent: u32::from(has_tangents),
            })
            .collect();
        let (bounds_min, bounds_extent) = bounds_vectors(bounds);
        let params = GpuSkinParams {
            bounds_min,
            bounds_extent,
            vertex_count: count as u32,
            matrix_count: required_matrices as u32,
            pad: [0; 2],
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skin_params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skin_bind_vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let skin_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skin_matrices"),
            size: required_matrices.max(1) as u64 * MATRIX_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skin_compute_bg"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bind_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: skin_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: out_vertices.as_entire_binding(),
                },
            ],
        });
        Some(Self {
            skin_buffer,
            bind_group,
            required_matrices,
            workgroups,
        })
    }

    /// Number of skin matrices the dispatch reads.
    pub(crate) fn required_matrices(&self) -> usize {
        self.required_matrices
    }

    /// Uploads `matrices` for the next dispatch. `false` if there are too few matrices.
    pub(crate) fn upload(&self, queue: &wgpu::Queue, matrices: &[Mat4]) -> bool {
        if matrices.len() < self.required_matrices {
            return false;
        }
        if self.required_matrices > 0 {
            let data: Vec<[f32; 16]> = matrices[..self.required_matrices]
                .iter()
                .map(|m| m.to_cols_array())
                .collect();
            queue.write_buffer(&self.skin_buffer, 0, bytemuck::cast_slice(&data));
        }
        true
    }

    /// Records this mesh's skinning dispatch into `pass` (the matrices must be uploaded).
    pub(crate) fn dispatch(&self, pass: &mut wgpu::ComputePass<'_>, pipeline: &SkinPipeline) {
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(self.workgroups, 1, 1);
    }

    /// Uploads `matrices` and submits one skinning dispatch. `false` if there are too few matrices.
    pub(crate) fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &SkinPipeline,
        matrices: &[Mat4],
    ) -> bool {
        run_batch(device, queue, pipeline, &[(self, matrices)]) == 1
    }
}

/// Uploads every entry's matrices, then records all dispatches into one compute pass of one
/// command buffer and submits it once. Entries with too few matrices are skipped; returns how
/// many were skinned.
pub(crate) fn run_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &SkinPipeline,
    items: &[(&GpuSkin, &[Mat4])],
) -> usize {
    let ready: Vec<&GpuSkin> = items
        .iter()
        .filter(|(skin, matrices)| skin.upload(queue, matrices))
        .map(|(skin, _)| *skin)
        .collect();
    if ready.is_empty() {
        return 0;
    }
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("skin_compute_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("skin_compute"),
            timestamp_writes: None,
        });
        for skin in &ready {
            skin.dispatch(&mut pass, pipeline);
        }
    }
    queue.submit(Some(encoder.finish()));
    ready.len()
}
