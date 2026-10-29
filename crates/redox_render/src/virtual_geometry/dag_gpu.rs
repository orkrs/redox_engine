//! GPU hierarchy traversal of DAG assets (VG-02).
//!
//! The cut of [`super::dag::cut`] is computed level by level: pass `k` consumes the
//! `(instance, cluster)` pairs of queue `k % 2`, emits the chosen clusters into the draw-command
//! buffer and queues the children of the clusters that are still too coarse into the other queue.
//! The dispatch size of every pass is written on the GPU by a one-thread `prepare_args` pass, so the
//! CPU never waits for a count during traversal.
//!
//! Hi-Z: clusters that the pyramid of the previous frame occludes are recorded as rejects; after
//! the current pyramid is built, [`DagTraversal::record_retest`] draws the rejects that are visible.
//!
//! Scope (v1):
//! - Instances are culled against the frustum and seeded with their roots on the CPU (the instance
//!   sphere test is in `runtime/dag.rs`); there is no `vg_instance_cull` shader yet.
//! - Cluster frustum tests use the sphere only; DAG clusters have no normal cone.
//! - Draw count: [`DagTraversal::poll_command_count`] returns the count of the previous frame.
//! - Binding budget: the traversal layout uses 8 storage buffers per stage, which is the WebGPU
//!   default. The device limit comes from `required_device_limits` (see `renderer/create.rs`);
//!   RFL-01 raises it, so up to 12 storage buffers per stage would be allowed after that change.
//! - Capacities (`max_clusters`, `max_child_ids`, `queue_capacity`) are fixed at construction.

use bytemuck::{Pod, Zeroable};

use super::dag::{ClusterDag, DagClusterGpu, DagInstance};
use super::gpu_cull::{CullParamsUniform, storage_read_entry, storage_rw_entry, uniform_entry};
use crate::shader::{VG_DAG_TRAVERSE_WGSL, create_shader_module};

/// Upper bound of traversal passes. `DagParams::max_levels` (24) needs at most 26.
pub const MAX_TRAVERSE_PASSES: u32 = 32;
/// Counter slot of the number of draw commands written by the traversal.
pub const COUNTER_CMD: usize = 2;
/// Counter slot of the number of queue overflows (clusters selected instead of their children).
pub const COUNTER_OVERFLOW: usize = 3;
/// Counter slot of the number of Hi-Z rejects recorded for the phase-2 retest.
pub const COUNTER_REJECT: usize = 4;
/// Number of counters in the counters buffer.
const COUNTER_COUNT: u64 = 5;
/// Bytes of one queue entry `(instance, cluster)`.
const QUEUE_ENTRY_BYTES: u64 = 8;
/// Bytes of one indirect dispatch argument block (`x, y, z`).
const DISPATCH_STRIDE: u64 = 12;
/// Bytes of one [`DagClusterGpu`].
const CLUSTER_BYTES: u64 = std::mem::size_of::<DagClusterGpu>() as u64;

/// GPU mirror of one placed DAG instance (112 bytes, `DagInstanceGpu` in WGSL).
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
    /// Added to the base vertex of every draw of this instance (vertices of the asset in the global buffer).
    pub vertex_base: u32,
    /// Padding to 112 bytes.
    pub _pad: u32,
}

const _: () = assert!(std::mem::size_of::<DagInstanceGpu>() == 112);

/// Per-pass uniform (32 bytes, `DagPassParams` in WGSL).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DagPassParams {
    /// Projection scale at 1 m (`screen_height / (2 * tan(fov_y / 2))`).
    pub pixels_per_unit: f32,
    /// Error threshold in pixels.
    pub threshold_px: f32,
    /// Capacity of each queue and of the reject list; entries beyond it are coarsened, never written.
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

/// Uniform of the `prepare_args` pass (16 bytes, `PrepareParams` in WGSL).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct PrepareParams {
    queue_capacity: u32,
    _pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<PrepareParams>() == 16);

/// Static hierarchy data of several DAG assets concatenated into one GPU layout.
#[derive(Clone, Debug, Default)]
pub struct DagSceneData {
    /// Cluster records of all hierarchies, in order. `index_offset` is global (already rebased);
    /// `first_child` indexes `child_ids`.
    ///
    /// GPU layout: `DagClusterGpu` (96 bytes) is read by `DagCluster` in `vg_dag_traverse.wgsl`
    /// as scalars in the same order as the Rust fields (`center`, `lod_center`, `parent_lod_center`
    /// as three f32 each). Changing the field order here requires changing the WGSL struct.
    pub clusters: Vec<DagClusterGpu>,
    /// Children of all groups, as global cluster ids.
    pub child_ids: Vec<u32>,
    /// Root cluster ids of all hierarchies, one after another (global ids).
    pub root_list: Vec<u32>,
    /// `(first, count)` of each hierarchy's roots in [`Self::root_list`].
    pub root_ranges: Vec<(u32, u32)>,
    /// Local bounding sphere (centre xyz, radius w) of each hierarchy's roots.
    pub bounds: Vec<[f32; 4]>,
    /// Vertex base of each hierarchy in the global vertex buffer.
    pub vertex_bases: Vec<u32>,
    /// Deepest level of any cluster (0 = leaves).
    pub max_level: u32,
}

impl DagSceneData {
    /// Concatenates the hierarchies. `bases[i]` is `(vertex_base, index_base)` of hierarchy `i`:
    /// the asset's triangle indices are stored at `index_base + DagCluster::index_offset` in the
    /// global index buffer, and the vertices of the asset start at `vertex_base`. Draws add the
    /// vertex base as `base_vertex`, so the stored indices stay local to the asset.
    ///
    /// Cluster ids of hierarchy `i` are offset by the number of clusters of the hierarchies before it.
    pub fn concat(dags: &[ClusterDag], bases: &[(u32, u32)]) -> Self {
        let mut out = Self::default();
        for (d, dag) in dags.iter().enumerate() {
            let (vertex_base, index_base) = bases.get(d).copied().unwrap_or((0, 0));
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
                let mut gpu = DagClusterGpu::from_cluster(c, child_base + first, count);
                gpu.index_offset += index_base;
                out.clusters.push(gpu);
                out.max_level = out.max_level.max(c.level);
                if c.parent_error == f32::INFINITY {
                    out.root_list.push(base + i as u32);
                }
            }
            let root_count = out.root_list.len() as u32 - root_first;
            out.root_ranges.push((root_first, root_count));
            let sphere = root_sphere(&out.clusters, &out.root_list[root_first as usize..]);
            out.bounds.push(sphere);
            out.vertex_bases.push(vertex_base);
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
    /// Instances to traverse, indexed by the `first_instance` they draw with. An entry whose
    /// `dag` is out of range is skipped.
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

/// Multi-pass GPU traversal of DAG hierarchies.
pub struct DagTraversal {
    traverse: wgpu::ComputePipeline,
    retest: wgpu::ComputePipeline,
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
    rejects: wgpu::Buffer,
    clusters: wgpu::Buffer,
    child_ids: wgpu::Buffer,
    count_staging: wgpu::Buffer,
    count_pending: bool,
    queue_capacity: u32,
    max_instances: u32,
    max_clusters: u32,
    max_child_ids: u32,
    scene: Option<DagSceneData>,
}

impl DagTraversal {
    /// Creates the pipelines and the fixed-size buffers.
    ///
    /// - `queue_capacity`: entries of each queue and of the reject list.
    /// - `max_instances`: instances per frame.
    /// - `max_clusters`, `max_child_ids`: size of the hierarchy buffers (see [`Self::upload_dags`]).
    pub fn new(
        device: &wgpu::Device,
        queue_capacity: u32,
        max_instances: u32,
        max_clusters: u32,
        max_child_ids: u32,
    ) -> Self {
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
                storage_rw_entry(11),
            ],
        });
        let bgl_prepare = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_dag_prepare_bgl"),
            entries: &[storage_rw_entry(7), storage_rw_entry(12), uniform_entry(13)],
        });

        let shader = create_shader_module(device, "vg_dag_traverse_shader", VG_DAG_TRAVERSE_WGSL);
        let traverse_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_dag_traverse_layout"),
            bind_group_layouts: &[Some(&bgl_traverse)],
            immediate_size: 0,
        });
        let traverse = create_pipeline(device, &shader, &traverse_layout, "main", "vg_dag_traverse");
        let retest =
            create_pipeline(device, &shader, &traverse_layout, "retest_rejects", "vg_dag_retest");
        let prepare_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_dag_prepare_layout"),
            bind_group_layouts: &[Some(&bgl_prepare)],
            immediate_size: 0,
        });
        let prepare = create_pipeline(device, &shader, &prepare_layout, "prepare_args", "vg_dag_prepare");

        let queue_bytes = u64::from(queue_capacity).max(1) * QUEUE_ENTRY_BYTES;
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let uniform = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        Self {
            traverse,
            retest,
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
            rejects: create_buffer(device, "vg_dag_rejects", queue_bytes, storage),
            clusters: create_buffer(
                device,
                "vg_dag_clusters",
                u64::from(max_clusters).max(1) * CLUSTER_BYTES,
                storage,
            ),
            child_ids: create_buffer(
                device,
                "vg_dag_child_ids",
                u64::from(max_child_ids).max(1) * 4,
                storage,
            ),
            count_staging: create_buffer(
                device,
                "vg_dag_count_staging",
                4,
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            ),
            count_pending: false,
            queue_capacity,
            max_instances,
            max_clusters,
            max_child_ids,
            scene: None,
        }
    }

    /// Uploads the static hierarchy of `dags`; instances refer to them by index.
    ///
    /// `bases[i]` is `(vertex_base, index_base)` of hierarchy `i` (see [`DagSceneData::concat`]).
    /// Fails, leaving the previous hierarchy in place, when the clusters or child ids exceed the
    /// capacities given to [`Self::new`].
    pub fn upload_dags(
        &mut self,
        queue: &wgpu::Queue,
        dags: &[ClusterDag],
        bases: &[(u32, u32)],
    ) -> Result<(), String> {
        let data = DagSceneData::concat(dags, bases);
        if data.clusters.len() > self.max_clusters as usize {
            return Err(format!(
                "{} clusters exceed the capacity {}",
                data.clusters.len(),
                self.max_clusters
            ));
        }
        if data.child_ids.len() > self.max_child_ids as usize {
            return Err(format!(
                "{} child ids exceed the capacity {}",
                data.child_ids.len(),
                self.max_child_ids
            ));
        }
        if data.clusters.is_empty() {
            self.scene = None;
            return Ok(());
        }
        queue.write_buffer(&self.clusters, 0, bytemuck::cast_slice(&data.clusters));
        if !data.child_ids.is_empty() {
            queue.write_buffer(&self.child_ids, 0, bytemuck::cast_slice(&data.child_ids));
        }
        self.scene = Some(data);
        Ok(())
    }

    /// Bounding sphere of hierarchy `dag` in its local space, if uploaded.
    pub fn dag_bound(&self, dag: usize) -> Option<[f32; 4]> {
        self.scene.as_ref()?.bounds.get(dag).copied()
    }

    /// Number of traversal passes needed by the uploaded hierarchies (0 when nothing is uploaded).
    pub fn pass_count(&self) -> u32 {
        self.scene
            .as_ref()
            .map_or(0, |s| (s.max_level + 2).min(MAX_TRAVERSE_PASSES))
    }

    /// Records the traversal of `frame` into `encoder`, writing draw commands into `commands`
    /// and Hi-Z rejects into the reject list. Returns the number of passes recorded
    /// (0 when no hierarchy is uploaded).
    ///
    /// The uniforms, the instance array and the queue seeds are written with
    /// `queue.write_buffer`, which the submission of `encoder` follows.
    pub fn record(
        &mut self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &DagFrame<'_>,
        cull: &CullParamsUniform,
        hiz: (&wgpu::TextureView, &wgpu::Sampler),
        commands: &wgpu::Buffer,
    ) -> u32 {
        let passes = self.pass_count();
        if passes == 0 {
            return 0;
        }
        self.write_frame(queue, frame, cull);

        let prepare_bg = self.prepare_bind_group(device);
        let traverse_bg = [
            self.traverse_bind_group(device, 0, hiz, commands),
            self.traverse_bind_group(device, 1, hiz, commands),
        ];

        self.dispatch_prepare(encoder, &prepare_bg);
        for level in 0..passes {
            let src = (level % 2) as usize;
            let dst = 1 - src;
            // The destination counter is consumed by the next pass only, so it is reset here.
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
        self.copy_command_count(encoder);
        self.count_pending = true;
        passes
    }

    /// Phase 2: re-tests the Hi-Z rejects recorded by the last [`Self::record`] against the pyramid
    /// built after it, and draws the ones that are visible. Records nothing when there are no
    /// hierarchies. Must follow `record` of the same frame.
    pub fn record_retest(
        &mut self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        cull: &CullParamsUniform,
        hiz: (&wgpu::TextureView, &wgpu::Sampler),
        commands: &wgpu::Buffer,
    ) {
        if self.scene.is_none() {
            return;
        }
        queue.write_buffer(&self.cull_params, 0, bytemuck::bytes_of(cull));
        let bind_group = self.traverse_bind_group(device, 0, hiz, commands);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("vg_dag_retest"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.retest);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(self.queue_capacity.div_ceil(64), 1, 1);
        }
        self.copy_command_count(encoder);
        self.count_pending = true;
    }

    /// Reads the draw-command count written by the last recorded frame, one frame late, as the
    /// flat path does. Returns `None` when no copy is pending.
    pub fn poll_command_count(&mut self, device: &wgpu::Device) -> Option<u32> {
        if !self.count_pending {
            return None;
        }
        let slice = self.count_staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = matches!(rx.try_recv(), Ok(Ok(())));
        let count = if mapped {
            slice
                .get_mapped_range()
                .ok()
                .and_then(|data| <[u8; 4]>::try_from(&data[0..4]).ok())
                .map(u32::from_le_bytes)
        } else {
            None
        };
        self.count_staging.unmap();
        self.count_pending = false;
        count
    }

    /// Writes the per-frame uniforms, the instance array and the queue seeds.
    fn write_frame(&self, queue: &wgpu::Queue, frame: &DagFrame<'_>, cull: &CullParamsUniform) {
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        let mut instances = Vec::with_capacity(frame.instances.len());
        let mut seeds: Vec<[u32; 2]> = Vec::new();
        for (ii, inst) in frame.instances.iter().take(self.max_instances as usize).enumerate() {
            let (root_begin, root_count) = scene
                .root_ranges
                .get(inst.dag)
                .copied()
                .unwrap_or((0, 0));
            let bound = scene.bounds.get(inst.dag).copied().unwrap_or([0.0; 4]);
            let vertex_base = scene.vertex_bases.get(inst.dag).copied().unwrap_or(0);
            let (local_eye, root_count) = match local_point(inst.transform, frame.eye) {
                Some(p) => ([p[0], p[1], p[2], 1.0], root_count),
                None => ([0.0; 4], 0),
            };
            instances.push(DagInstanceGpu {
                transform: inst.transform,
                local_eye,
                bound,
                root_begin,
                root_count,
                vertex_base,
                _pad: 0,
            });
            if root_count == 0 {
                continue;
            }
            let first = root_begin as usize;
            for &root in &scene.root_list[first..first + root_count as usize] {
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
        // Queue A holds the seeds; the other counters start at 0 for this frame.
        let counters: [u32; COUNTER_COUNT as usize] = [seeds.len() as u32, 0, 0, 0, 0];
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

    fn copy_command_count(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_buffer_to_buffer(
            &self.counters,
            (COUNTER_CMD * 4) as u64,
            &self.count_staging,
            0,
            4,
        );
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
                    binding: 12,
                    resource: self.dispatch_args.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: self.prepare_params.as_entire_binding(),
                },
            ],
        })
    }

    /// Bind group of the pass that reads queue `src` (0 = A, 1 = B) and writes the other one.
    /// The phase-2 retest uses the group with `src = 0`; it reads only the reject list.
    fn traverse_bind_group(
        &self,
        device: &wgpu::Device,
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
                    resource: self.clusters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.child_ids.as_entire_binding(),
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
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: self.rejects.as_entire_binding(),
                },
            ],
        })
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
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
///
/// The same routine as `world_to_local` in `dag/gpu.rs`, which is not shared because that module
/// is outside this change.
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
