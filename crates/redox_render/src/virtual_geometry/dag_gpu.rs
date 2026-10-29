//! GPU hierarchy traversal of DAG assets (VG-02).
//!
//! The cut of [`super::dag::cut`] is computed level by level: pass `k` consumes the
//! `(instance, cluster)` pairs of queue `k % 2`, emits the chosen clusters into the draw-command
//! buffer and queues the children of the clusters that are still too coarse into the other queue.
//! The dispatch size of every pass is written on the GPU by a one-thread `prepare_args` pass, so the
//! CPU never waits for a count during traversal.

use bytemuck::{Pod, Zeroable};

use super::dag::{ClusterDag, DagClusterGpu, DagInstance};
use super::gpu_cull::{CullParamsUniform, storage_read_entry, storage_rw_entry, uniform_entry};
use crate::shader::{VG_DAG_TRAVERSE_WGSL, create_shader_module};

/// Upper bound of traversal passes. `DagParams::max_levels` (24) needs at most 26.
pub const MAX_TRAVERSE_PASSES: u32 = 32;
/// Counter slot of the number of draw commands written by the traversal.
pub const COUNTER_CMD: usize = 2;
/// Counter slot of the number of queue overflows (clusters drawn instead of their children).
pub const COUNTER_OVERFLOW: usize = 3;
/// Number of counters in the counters buffer.
const COUNTER_COUNT: u64 = 4;
/// Bytes of one queue entry `(instance, cluster)`.
const QUEUE_ENTRY_BYTES: u64 = 8;
/// Bytes of one indirect dispatch argument block (`x, y, z`).
const DISPATCH_STRIDE: u64 = 12;

/// GPU mirror of one placed DAG instance (112 bytes, see `DagInstance` in WGSL).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DagInstanceGpu {
    /// Local-to-world transform, column-major.
    pub transform: [[f32; 4]; 4],
    /// Eye position in the local space of the instance (`w` unused).
    pub local_eye: [f32; 4],
    /// Local bounding sphere of the hierarchy: centre (xyz) and radius (w).
    pub bound: [f32; 4],
    /// First entry of the instance's roots in the root list.
    pub root_begin: u32,
    /// Number of roots of the instance (0 when the instance is skipped).
    pub root_count: u32,
    /// Padding to 112 bytes.
    pub _pad: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<DagInstanceGpu>() == 112);

/// Per-pass uniform (32 bytes, see `DagPassParams` in WGSL).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DagPassParams {
    /// Projection scale at 1 m (`screen_height / (2 * tan(fov_y / 2))`).
    pub pixels_per_unit: f32,
    /// Error threshold in pixels.
    pub threshold_px: f32,
    /// Capacity of each queue; entries beyond it are coarsened, never written.
    pub queue_capacity: u32,
    /// Non-zero enables the frustum test of selected clusters.
    pub cull_frustum: u32,
    /// Queue read by this pass (0 = A, 1 = B).
    pub src_slot: u32,
    /// Queue written by this pass (0 = A, 1 = B).
    pub dst_slot: u32,
    /// Padding to 32 bytes.
    pub _pad: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<DagPassParams>() == 32);

/// Uniform of the `prepare_args` pass (16 bytes, see `PrepareParams` in WGSL).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct PrepareParams {
    queue_capacity: u32,
    _pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<PrepareParams>() == 16);

/// Static hierarchy data of several DAG assets, concatenated into one GPU layout.
#[derive(Clone, Debug, Default)]
pub struct DagSceneData {
    /// Cluster records of all hierarchies; `first_child` indexes `child_ids`.
    pub clusters: Vec<DagClusterGpu>,
    /// Children of all groups, as global cluster ids.
    pub child_ids: Vec<u32>,
    /// Root cluster ids of all hierarchies, one after another (global ids).
    pub root_list: Vec<u32>,
    /// `(first, count)` of each hierarchy's roots in [`Self::root_list`].
    pub root_ranges: Vec<(u32, u32)>,
    /// Local bounding sphere (centre xyz, radius w) of each hierarchy's roots.
    pub bounds: Vec<[f32; 4]>,
    /// Deepest level of any cluster (0 = leaves).
    pub max_level: u32,
}

impl DagSceneData {
    /// Concatenates the hierarchies. Cluster ids of the `i`-th hierarchy are offset by the
    /// number of clusters of the hierarchies before it.
    pub fn concat(dags: &[ClusterDag]) -> Self {
        let mut out = Self::default();
        for dag in dags {
            let base = out.clusters.len() as u32;
            let child_base = out.child_ids.len() as u32;
            let (children, ranges) = dag.children_index();
            out.child_ids.extend(children.iter().map(|&c| c + base));

            let root_first = out.root_list.len() as u32;
            for (i, c) in dag.clusters.iter().enumerate() {
                let (first, count) = match c.group {
                    u32::MAX => (0, 0),
                    g => ranges.get(g as usize).copied().unwrap_or((0, 0)),
                };
                out.clusters
                    .push(DagClusterGpu::from_cluster(c, child_base + first, count));
                out.max_level = out.max_level.max(c.level);
                if c.parent_error == f32::INFINITY {
                    out.root_list.push(base + i as u32);
                }
            }
            let root_count = out.root_list.len() as u32 - root_first;
            out.root_ranges.push((root_first, root_count));
            let sphere = root_sphere(&out.clusters, &out.root_list[root_first as usize..]);
            out.bounds.push(sphere);
        }
        out
    }
}

/// Bounding sphere of the given clusters (centre is the mean of their centres).
fn root_sphere(clusters: &[DagClusterGpu], roots: &[u32]) -> [f32; 4] {
    let picked: Vec<&DagClusterGpu> = roots
        .iter()
        .filter_map(|&r| clusters.get(r as usize))
        .collect();
    if picked.is_empty() {
        return [0.0; 4];
    }
    let n = picked.len() as f32;
    let mut centre = [0.0f32; 3];
    for c in &picked {
        for (axis, value) in centre.iter_mut().enumerate() {
            *value += c.center[axis] / n;
        }
    }
    let radius = picked
        .iter()
        .map(|c| {
            let d = [
                c.center[0] - centre[0],
                c.center[1] - centre[1],
                c.center[2] - centre[2],
            ];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() + c.radius
        })
        .fold(0.0f32, f32::max);
    [centre[0], centre[1], centre[2], radius]
}

/// Input of one traversal: the placed instances and the view.
#[derive(Clone, Copy, Debug)]
pub struct DagFrame<'a> {
    /// Instances to traverse; `DagInstance::dag` indexes the hierarchies given to
    /// [`DagTraversal::upload_dags`].
    pub instances: &'a [DagInstance],
    /// Eye position in world space.
    pub eye: [f32; 3],
    /// Projection scale at 1 m (see [`DagPassParams::pixels_per_unit`]).
    pub pixels_per_unit: f32,
    /// Error threshold in pixels.
    pub threshold_px: f32,
    /// Whether selected clusters are tested against the frustum planes of the [`CullParamsUniform`].
    pub cull_frustum: bool,
}

/// Uploaded hierarchy buffers and their CPU-side description.
struct DagSceneGpu {
    clusters: wgpu::Buffer,
    child_ids: wgpu::Buffer,
    data: DagSceneData,
}

/// Multi-pass GPU traversal of DAG hierarchies.
pub struct DagTraversal {
    traverse: wgpu::ComputePipeline,
    prepare: wgpu::ComputePipeline,
    bgl_traverse: wgpu::BindGroupLayout,
    bgl_prepare: wgpu::BindGroupLayout,
    queue_a: wgpu::Buffer,
    queue_b: wgpu::Buffer,
    counters: wgpu::Buffer,
    dispatch_args: wgpu::Buffer,
    prepare_params: wgpu::Buffer,
    pass_params: [wgpu::Buffer; 2],
    cull_params: wgpu::Buffer,
    instances: wgpu::Buffer,
    queue_capacity: u32,
    max_instances: u32,
    scene: Option<DagSceneGpu>,
}

impl DagTraversal {
    /// Creates the pipelines and the fixed-size queue buffers.
    ///
    /// `queue_capacity` bounds each queue (entries), `max_instances` the instances per frame.
    pub fn new(device: &wgpu::Device, queue_capacity: u32, max_instances: u32) -> Self {
        let bgl_traverse = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_dag_traverse_bgl"),
            entries: &[
                uniform_entry(0),
                uniform_entry(1),
                storage_read_entry(2),
                storage_read_entry(3),
                storage_read_entry(4),
                storage_read_entry(5),
                storage_rw_entry(6),
                storage_rw_entry(7),
                storage_rw_entry(8),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });
        let bgl_prepare = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_dag_prepare_bgl"),
            entries: &[
                storage_rw_entry(7),
                storage_rw_entry(11),
                uniform_entry(12),
            ],
        });

        let shader = create_shader_module(device, "vg_dag_traverse_shader", VG_DAG_TRAVERSE_WGSL);
        let traverse_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_dag_traverse_layout"),
            bind_group_layouts: &[Some(&bgl_traverse)],
            immediate_size: 0,
        });
        let traverse = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("vg_dag_traverse_pipeline"),
            layout: Some(&traverse_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
        });
        let prepare_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_dag_prepare_layout"),
            bind_group_layouts: &[Some(&bgl_prepare)],
            immediate_size: 0,
        });
        let prepare = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("vg_dag_prepare_pipeline"),
            layout: Some(&prepare_layout),
            module: &shader,
            entry_point: Some("prepare_args"),
            compilation_options: Default::default(),
        });

        let queue_bytes = u64::from(queue_capacity).max(1) * QUEUE_ENTRY_BYTES;
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let uniform = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        Self {
            traverse,
            prepare,
            bgl_traverse,
            bgl_prepare,
            queue_a: create_buffer(device, "vg_dag_queue_a", queue_bytes, storage),
            queue_b: create_buffer(device, "vg_dag_queue_b", queue_bytes, storage),
            counters: create_buffer(
                device,
                "vg_dag_counters",
                COUNTER_COUNT * 4,
                storage | wgpu::BufferUsages::COPY_SRC,
            ),
            dispatch_args: create_buffer(
                device,
                "vg_dag_dispatch_args",
                DISPATCH_STRIDE * 2,
                storage | wgpu::BufferUsages::INDIRECT,
            ),
            prepare_params: create_buffer(
                device,
                "vg_dag_prepare_params",
                std::mem::size_of::<PrepareParams>() as u64,
                uniform,
            ),
            pass_params: [
                create_buffer(
                    device,
                    "vg_dag_pass_params_a",
                    std::mem::size_of::<DagPassParams>() as u64,
                    uniform,
                ),
                create_buffer(
                    device,
                    "vg_dag_pass_params_b",
                    std::mem::size_of::<DagPassParams>() as u64,
                    uniform,
                ),
            ],
            cull_params: create_buffer(
                device,
                "vg_dag_cull_params",
                std::mem::size_of::<CullParamsUniform>() as u64,
                uniform,
            ),
            instances: create_buffer(
                device,
                "vg_dag_instances",
                u64::from(max_instances).max(1) * std::mem::size_of::<DagInstanceGpu>() as u64,
                storage,
            ),
            queue_capacity,
            max_instances,
            scene: None,
        }
    }

    /// Uploads the static hierarchy of `dags`. Instances refer to them by index.
    pub fn upload_dags(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, dags: &[ClusterDag]) {
        let data = DagSceneData::concat(dags);
        if data.clusters.is_empty() {
            self.scene = None;
            return;
        }
        let clusters_bytes: &[u8] = bytemuck::cast_slice(&data.clusters);
        let clusters = create_buffer(
            device,
            "vg_dag_clusters",
            clusters_bytes.len() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        queue.write_buffer(&clusters, 0, clusters_bytes);

        let child_bytes: &[u8] = bytemuck::cast_slice(&data.child_ids);
        let child_ids = create_buffer(
            device,
            "vg_dag_child_ids",
            (child_bytes.len() as u64).max(4),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        if !child_bytes.is_empty() {
            queue.write_buffer(&child_ids, 0, child_bytes);
        }
        self.scene = Some(DagSceneGpu {
            clusters,
            child_ids,
            data,
        });
    }

    /// Number of traversal passes needed by the uploaded hierarchies (0 when nothing is uploaded).
    pub fn pass_count(&self) -> u32 {
        self.scene
            .as_ref()
            .map_or(0, |s| (s.data.max_level + 2).min(MAX_TRAVERSE_PASSES))
    }

    /// Buffer with the counters `[queue A, queue B, draw commands, overflows]` after a recorded frame.
    pub fn counters_buffer(&self) -> &wgpu::Buffer {
        &self.counters
    }

    /// Records the traversal of `frame` into `encoder`, writing draw commands into `commands`.
    ///
    /// Returns the number of passes recorded (0 when no hierarchy is uploaded). The uniform
    /// buffers and the instance and queue seeds are written with `queue.write_buffer`, so they are
    /// visible to the encoded passes in submission order.
    pub fn record(
        &self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &DagFrame<'_>,
        cull: &CullParamsUniform,
        hiz: (&wgpu::TextureView, &wgpu::Sampler),
        commands: &wgpu::Buffer,
    ) -> u32 {
        let Some(scene) = self.scene.as_ref() else {
            return 0;
        };
        self.write_frame(queue, scene, frame, cull);

        let prepare_bg = self.prepare_bind_group(device);
        let traverse_bg = [
            self.traverse_bind_group(device, scene, 0, hiz, commands),
            self.traverse_bind_group(device, scene, 1, hiz, commands),
        ];
        let passes = self.pass_count();

        self.dispatch_prepare(encoder, &prepare_bg);
        for level in 0..passes {
            let src = (level % 2) as usize;
            let dst = 1 - src;
            // The destination counter is consumed by this pass's successor only, so it is reset here.
            encoder.clear_buffer(&self.counters, (dst * 4) as u64, Some(4));
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("vg_dag_traverse"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.traverse);
                pass.set_bind_group(0, &traverse_bg[src], &[]);
                pass.dispatch_workgroups_indirect(&self.dispatch_args, src as u64 * DISPATCH_STRIDE);
            }
            self.dispatch_prepare(encoder, &prepare_bg);
        }
        passes
    }

    /// Writes the per-frame uniforms, the instance array, the queue seeds and the counters.
    fn write_frame(
        &self,
        queue: &wgpu::Queue,
        scene: &DagSceneGpu,
        frame: &DagFrame<'_>,
        cull: &CullParamsUniform,
    ) {
        let mut instances = Vec::with_capacity(frame.instances.len());
        let mut seeds: Vec<[u32; 2]> = Vec::new();
        for (ii, inst) in frame.instances.iter().take(self.max_instances as usize).enumerate() {
            let (root_begin, root_count) = scene
                .data
                .root_ranges
                .get(inst.dag)
                .copied()
                .unwrap_or((0, 0));
            let bound = scene.data.bounds.get(inst.dag).copied().unwrap_or([0.0; 4]);
            let local = local_point(inst.transform, frame.eye);
            let (local_eye, root_count) = match local {
                Some(p) => ([p[0], p[1], p[2], 1.0], root_count),
                None => ([0.0; 4], 0),
            };
            instances.push(DagInstanceGpu {
                transform: inst.transform,
                local_eye,
                bound,
                root_begin,
                root_count,
                _pad: [0; 2],
            });
            if root_count == 0 {
                continue;
            }
            let first = root_begin as usize;
            for &root in &scene.data.root_list[first..first + root_count as usize] {
                seeds.push([ii as u32, root]);
            }
        }

        if seeds.len() > self.queue_capacity as usize {
            log::warn!(
                "[VG DAG] {} root seeds exceed the queue capacity {}; the rest is not drawn",
                seeds.len(),
                self.queue_capacity
            );
            seeds.truncate(self.queue_capacity as usize);
        }

        if !instances.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        }
        if !seeds.is_empty() {
            queue.write_buffer(&self.queue_a, 0, bytemuck::cast_slice(&seeds));
        }
        // Queue A holds the seeds; queue B, both draw-command counters and the overflow counter start at 0.
        let counters = [seeds.len() as u32, 0, 0, 0];
        queue.write_buffer(&self.counters, 0, bytemuck::cast_slice(&counters));
        queue.write_buffer(&self.cull_params, 0, bytemuck::bytes_of(cull));

        let pass = |src: u32, dst: u32| DagPassParams {
            pixels_per_unit: frame.pixels_per_unit,
            threshold_px: frame.threshold_px,
            queue_capacity: self.queue_capacity,
            cull_frustum: u32::from(frame.cull_frustum),
            src_slot: src,
            dst_slot: dst,
            _pad: [0; 2],
        };
        queue.write_buffer(&self.pass_params[0], 0, bytemuck::bytes_of(&pass(0, 1)));
        queue.write_buffer(&self.pass_params[1], 0, bytemuck::bytes_of(&pass(1, 0)));
        let prepare = PrepareParams {
            queue_capacity: self.queue_capacity,
            _pad: [0; 3],
        };
        queue.write_buffer(&self.prepare_params, 0, bytemuck::bytes_of(&prepare));
    }

    fn dispatch_prepare(&self, encoder: &mut wgpu::CommandEncoder, bind_group: &wgpu::BindGroup) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("vg_dag_prepare_args"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.prepare);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    fn prepare_bind_group(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_dag_prepare_bg"),
            layout: &self.bgl_prepare,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.counters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: self.dispatch_args.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: self.prepare_params.as_entire_binding(),
                },
            ],
        })
    }

    /// Bind group of the pass that reads queue `src` (0 = A, 1 = B) and writes the other one.
    fn traverse_bind_group(
        &self,
        device: &wgpu::Device,
        scene: &DagSceneGpu,
        src: usize,
        hiz: (&wgpu::TextureView, &wgpu::Sampler),
        commands: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let (src_queue, dst_queue) = if src == 0 {
            (&self.queue_a, &self.queue_b)
        } else {
            (&self.queue_b, &self.queue_a)
        };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_dag_traverse_bg"),
            layout: &self.bgl_traverse,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.cull_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.pass_params[src].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scene.clusters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: scene.child_ids.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: src_queue.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: dst_queue.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.counters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: commands.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(hiz.0),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(hiz.1),
                },
            ],
        })
    }
}

fn create_buffer(
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

/// Maps a world point into the local space of an affine transform (`None` if singular).
fn local_point(t: [[f32; 4]; 4], p: [f32; 3]) -> Option<[f32; 3]> {
    let m = [
        [t[0][0], t[1][0], t[2][0]],
        [t[0][1], t[1][1], t[2][1]],
        [t[0][2], t[1][2], t[2][2]],
    ];
    let b = [p[0] - t[3][0], p[1] - t[3][1], p[2] - t[3][2]];
    solve_3x3(m, b)
}

fn det3(m: [[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Solves `m * x = b` with Cramer's rule (`None` when `m` is singular).
fn solve_3x3(m: [[f32; 3]; 3], b: [f32; 3]) -> Option<[f32; 3]> {
    let det = det3(m);
    if det.abs() < 1e-12 {
        return None;
    }
    let mut x = [0.0f32; 3];
    for (col, value) in x.iter_mut().enumerate() {
        let mut mc = m;
        for (row, r) in mc.iter_mut().enumerate() {
            r[col] = b[row];
        }
        *value = det3(mc) / det;
    }
    Some(x)
}
