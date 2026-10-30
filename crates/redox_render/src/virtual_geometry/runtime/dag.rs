//! GPU-cull recording of the VG runtime: the two-phase Hi-Z cull (phase 1 on the previous pyramid,
//! phase 2 re-tests the rejects against the current one) and the readback of the draw count. The
//! flat meshlet path and the DAG path (see `dag_gpu`) share this recording.

//! DAG virtual geometry on the VG runtime (VG-02): registration of DAG assets, DAG instances, the
//! GPU traversal and its draw path, next to the flat meshlet path (mixed scenes are allowed).
//!
//! v1 limits: instances are culled against the frustum on the CPU and seeded with their roots on
//! the CPU (no `vg_instance_cull` shader); the draw count is read one frame late, like the flat path.
//! Capacities are constants below, because `VGConfig` is outside this change.

use std::collections::HashMap;

use crate::mesh::{CompressedVertex, bounds_vectors, compress_vertices};
use crate::virtual_geometry::culling::{Frustum, max_scale, transform_point};
use crate::virtual_geometry::dag::{ClusterDag, DagInstance, VGDagAsset};
use crate::virtual_geometry::dag_gpu::DagFrame;
use crate::virtual_geometry::dag_gpu::DagTraversal;
use crate::virtual_geometry::gpu_cull::CullParamsUniform;
use crate::virtual_geometry::indirect::IndirectDrawBuffer;
use crate::virtual_geometry::meshlet::VGInstanceData;

use super::{VGAssetId, VGInstanceHandle, VGSystem};

/// Clusters of all DAG assets together.
const DAG_MAX_CLUSTERS: usize = 1 << 16;
/// Child ids of all DAG assets together.
const DAG_MAX_CHILD_IDS: usize = 1 << 17;
/// Entries of each traversal queue and of the Hi-Z reject list.
const DAG_QUEUE_CAPACITY: u32 = 1 << 15;
/// Identity transform for slots that hold no DAG instance this frame.
const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Where a registered DAG asset lives in the global buffers.
#[derive(Clone, Copy, Debug)]
struct DagStoredAsset {
    /// Index of the hierarchy in [`DagState::hierarchies`].
    dag_index: usize,
    /// Local bounds, as the flat path stores them for vertex decoding.
    bounds_min: [f32; 4],
    bounds_extent: [f32; 4],
}

/// DAG-path state of [`VGSystem`].
pub(super) struct DagState {
    traversal: DagTraversal,
    /// Draw commands of the traversal; drawn with the same vis state as the flat path.
    indirect: IndirectDrawBuffer,
    /// Hierarchies of all registered DAG assets, in registration order.
    hierarchies: Vec<ClusterDag>,
    /// `(vertex_base, index_base)` of every hierarchy.
    bases: Vec<(u32, u32)>,
    assets: HashMap<VGAssetId, DagStoredAsset>,
    /// Live DAG instances: `(slot in instance_data, index of the hierarchy)`.
    instances: HashMap<VGInstanceHandle, (usize, usize)>,
}

impl DagState {
    /// Creates the traversal and the draw-command buffer.
    pub(super) fn new(device: &wgpu::Device, max_instances: usize, max_commands: usize) -> Self {
        Self {
            traversal: DagTraversal::new(
                device,
                DAG_QUEUE_CAPACITY,
                max_instances as u32,
                DAG_MAX_CLUSTERS as u32,
                DAG_MAX_CHILD_IDS as u32,
            ),
            indirect: IndirectDrawBuffer::new(device, max_commands, "vg_dag_indirect_draw"),
            hierarchies: Vec::new(),
            bases: Vec::new(),
            assets: HashMap::new(),
            instances: HashMap::new(),
        }
    }
}

/// Whether the sphere of `bound` (local space, radius in w) placed with `transform` meets the frustum.
fn sphere_in_frustum(frustum: &Frustum, transform: [[f32; 4]; 4], bound: [f32; 4]) -> bool {
    let centre = transform_point(&transform, [bound[0], bound[1], bound[2]]);
    let radius = bound[3] * max_scale(&transform);
    frustum.planes.iter().all(|p| {
        let d = p[0] * centre[0] + p[1] * centre[1] + p[2] * centre[2] + p[3];
        d >= -radius
    })
}

impl VGSystem {
    /// Registers a DAG asset: its vertices and triangle indices are appended to the global buffers
    /// and its hierarchy joins the traversal. Returns `None` when a buffer or a DAG capacity is full.
    ///
    /// Stored triangle indices stay local to the asset; draws add the asset's vertex base as
    /// `base_vertex`. The cluster `index_offset` in GPU data is rebased by the index base.
    pub fn register_dag_asset(
        &mut self,
        asset: VGDagAsset,
        queue: &wgpu::Queue,
    ) -> Option<VGAssetId> {
        let vert_needed = asset.vertices.len();
        let idx_needed = asset.indices.len();
        if self.vertex_watermark as usize + vert_needed > self.config.max_vertices
            || self.index_watermark as usize + idx_needed > self.config.max_indices
        {
            log::warn!("[VGSystem] Not enough GPU buffer space for a DAG asset");
            return None;
        }

        let vertex_base = self.vertex_watermark;
        let index_base = self.index_watermark;
        let dag_index = self.dag.hierarchies.len();
        self.dag.hierarchies.push(asset.dag);
        self.dag.bases.push((vertex_base, index_base));
        if self
            .dag
            .traversal
            .upload_dags(queue, &self.dag.hierarchies, &self.dag.bases)
            .is_err()
        {
            log::warn!("[VGSystem] DAG hierarchy capacity is full");
            self.dag.hierarchies.pop();
            self.dag.bases.pop();
            return None;
        }

        let aabb = redox_math::Aabb {
            min: redox_math::Vec3::from_array(asset.aabb[0]),
            max: redox_math::Vec3::from_array(asset.aabb[1]),
        };
        let compressed = compress_vertices(&asset.vertices, &aabb);
        let (bounds_min, bounds_extent) = bounds_vectors(&aabb);
        queue.write_buffer(
            &self.vertex_buffer,
            u64::from(vertex_base) * std::mem::size_of::<CompressedVertex>() as u64,
            bytemuck::cast_slice(&compressed),
        );
        queue.write_buffer(
            &self.index_buffer,
            u64::from(index_base) * std::mem::size_of::<u32>() as u64,
            bytemuck::cast_slice(&asset.indices),
        );
        self.vertex_watermark += vert_needed as u32;
        self.index_watermark += idx_needed as u32;

        let id = VGAssetId(self.next_asset_id);
        self.next_asset_id += 1;
        self.dag.assets.insert(
            id,
            DagStoredAsset {
                dag_index,
                bounds_min,
                bounds_extent,
            },
        );
        Some(id)
    }

    /// Spawns an instance of a DAG asset. Its slot also holds a `VGInstanceData` (without meshlets)
    /// so the vis pass finds the transform and the vertex decoding bounds by `first_instance`.
    pub fn spawn_dag_instance(
        &mut self,
        asset_id: VGAssetId,
        transform: [[f32; 4]; 4],
    ) -> Option<VGInstanceHandle> {
        let stored = *self.dag.assets.get(&asset_id)?;
        if self.instance_data.len() >= self.config.max_instances {
            log::warn!("[VGSystem] Max instances reached");
            return None;
        }
        let slot = self.find_free_instance_slot();
        let handle = VGInstanceHandle(self.next_instance_id);
        self.next_instance_id += 1;
        let inst = VGInstanceData {
            transform,
            meshlet_offset: 0,
            meshlet_count: 0,
            flags: 0,
            _pad: 0,
            bounds_min: stored.bounds_min,
            bounds_extent: stored.bounds_extent,
        };
        if slot < self.instance_data.len() {
            self.instance_data[slot] = inst;
        } else {
            self.instance_data.push(inst);
        }
        self.instance_map.insert(handle, slot);
        self.dag.instances.insert(handle, (slot, stored.dag_index));
        self.instances_dirty = true;
        Some(handle)
    }

    /// Draw commands the DAG traversal of the last frame produced (read back one frame late).
    pub fn dag_draw_count(&self) -> u32 {
        self.dag.indirect.count
    }

    /// Number of live DAG instances (the frame runs the traversal only when there are some).
    pub fn dag_instance_count(&self) -> usize {
        self.dag.instances.len()
    }

    /// Frees the DAG bookkeeping of an instance (called by `despawn_instance`).
    pub(super) fn forget_dag_instance(&mut self, handle: VGInstanceHandle) {
        self.dag.instances.remove(&handle);
    }

    /// Records the DAG traversal of this frame. The draw count is the one read back from the
    /// previous frame. `pixels_per_unit` is the projection scale at 1 m, `threshold_px` the error
    /// threshold (both the same as the CPU reference uses).
    pub fn record_dag_traverse(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
        pixels_per_unit: f32,
        threshold_px: f32,
    ) {
        if !self.config.dag_traverse {
            return;
        }
        // The vis pass reads the transform of every instance: upload them even when the host does
        // not call `prepare_frame` (scenes with DAG assets only).
        if self.instances_dirty && !self.instance_data.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instance_data),
            );
            self.instances_dirty = false;
        }
        if let Some(count) = self.dag.traversal.poll_command_count(device) {
            self.dag.indirect.count = (count as usize).min(self.dag.indirect.capacity) as u32;
        }
        if self.dag.instances.is_empty() {
            self.dag.indirect.count = 0;
            return;
        }
        let frustum = Frustum::from_view_proj(view_proj);
        let instances = self.dag_frame_instances(&frustum);
        let cull = self.cull_params(view_proj, camera_pos, hiz, 0);
        let frame = DagFrame {
            instances: &instances,
            eye: [camera_pos.x, camera_pos.y, camera_pos.z],
            pixels_per_unit,
            threshold_px,
            cull_frustum: true,
        };
        self.dag.traversal.record(
            queue,
            device,
            encoder,
            &frame,
            &cull,
            (hiz.view(0), hiz.sampler()),
            &self.dag.indirect.buffer,
        );
    }

    /// Phase 2 of the DAG path: re-tests the Hi-Z rejects of this frame against the pyramid built
    /// after [`Self::record_dag_traverse`]. Call it at the same place as `record_gpu_cull_phase2`.
    pub fn record_dag_phase2(
        &mut self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
    ) {
        if !self.config.dag_traverse || self.dag.instances.is_empty() {
            return;
        }
        let cull = self.cull_params(view_proj, camera_pos, hiz, 1);
        self.dag.traversal.record_retest(
            queue,
            device,
            encoder,
            &cull,
            (hiz.view(0), hiz.sampler()),
            &self.dag.indirect.buffer,
        );
    }

    /// Issues the DAG draws with the vis pass state, as [`Self::draw_vis_indirect`] does for flat draws.
    pub fn draw_dag_vis_indirect<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        vis: &'a crate::pass::vg_resolve::VgVisPass,
        frame_bg: &'a wgpu::BindGroup,
    ) {
        if !self.config.dag_traverse || self.dag.indirect.count == 0 {
            return;
        }
        crate::pass::vg_resolve::bind_vis_draw_state(
            pass,
            vis,
            frame_bg,
            &self.instances_bind_group,
            &self.vertex_buffer,
            &self.index_buffer,
        );
        self.dag.indirect.issue_draws(pass);
    }

    /// One entry per slot of `instance_data`: live DAG instances that meet the frustum, the rest
    /// skipped (`dag` out of range), so that the traversal's `first_instance` is the slot.
    fn dag_frame_instances(&self, frustum: &Frustum) -> Vec<DagInstance> {
        let mut out = vec![
            DagInstance {
                dag: usize::MAX,
                transform: IDENTITY,
            };
            self.instance_data.len()
        ];
        for (&handle, &(slot, dag)) in &self.dag.instances {
            if self.instance_map.get(&handle) != Some(&slot) {
                continue;
            }
            let transform = self.instance_data[slot].transform;
            if let Some(bound) = self.dag.traversal.dag_bound(dag)
                && !sphere_in_frustum(frustum, transform, bound)
            {
                continue;
            }
            out[slot] = DagInstance { dag, transform };
        }
        out
    }
}

impl VGSystem {
    /// Poll deferred GPU-cull readback (uses last frame's count for draws).
    pub fn poll_gpu_cull_count(&mut self, device: &wgpu::Device) {
        self.gpu_cull.poll_readback(device, &mut self.indirect);
    }

    /// Record GPU cull into the main frame encoder (no CPU stall).
    pub fn record_gpu_cull(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
    ) {
        self.upload_meshlets_if_dirty(queue);
        let params = self.cull_params(view_proj, camera_pos, hiz, 0);
        let inst_count = self.instance_data.len() as u32;
        self.gpu_cull.record_dispatch(
            device,
            encoder,
            queue,
            &params,
            &self.instance_buffer,
            &self.meshlet_gpu_buffer,
            &self.indirect,
            inst_count,
            hiz.view(0),
            hiz.sampler(),
            true,
        );
    }

    /// Phase 2 late cull: re-test Hi-Z rejects against the current-frame pyramid.
    pub fn record_gpu_cull_phase2(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
    ) {
        if !self.config.gpu_cull || self.active_instance_count() == 0 {
            return;
        }
        let params = self.cull_params(view_proj, camera_pos, hiz, 1);
        let inst_count = self.instance_data.len() as u32;
        self.gpu_cull.record_dispatch(
            device,
            encoder,
            queue,
            &params,
            &self.instance_buffer,
            &self.meshlet_gpu_buffer,
            &self.indirect,
            inst_count,
            hiz.view(0),
            hiz.sampler(),
            false,
        );
    }

    fn cull_params(
        &self,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
        cull_mode: u32,
    ) -> CullParamsUniform {
        let frustum = Frustum::from_view_proj(view_proj);
        CullParamsUniform {
            planes: frustum.planes,
            camera_pos: [camera_pos.x, camera_pos.y, camera_pos.z, 1.0],
            view_proj: *view_proj,
            hiz_size: [hiz.width as f32, hiz.height as f32],
            hiz_mip_count: hiz.mip_count,
            hiz_enabled: u32::from(hiz.is_ready()),
            cull_mode,
            reject_capacity: self.gpu_cull.reject_capacity(),
            _pad: [0; 2],
        }
    }

    /// Schedule async count readback on the main encoder.
    pub fn schedule_gpu_cull_readback(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.gpu_cull.schedule_readback(encoder);
    }

    /// Begin async map after submit.
    pub fn kick_gpu_cull_readback(&mut self) {
        self.gpu_cull.kick_readback();
    }

    /// Run GPU cull (requires `VGConfig::gpu_cull` and a pre-built encoder).
    #[allow(dead_code)]
    pub fn run_gpu_cull(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
    ) -> u32 {
        self.upload_meshlets_if_dirty(queue);
        let params = self.cull_params(view_proj, camera_pos, hiz, 0);
        let inst_count = self.instance_data.len() as u32;
        let mut enc = encoder;
        self.gpu_cull.record_dispatch(
            device,
            &mut enc,
            queue,
            &params,
            &self.instance_buffer,
            &self.meshlet_gpu_buffer,
            &self.indirect,
            inst_count,
            hiz.view(0),
            hiz.sampler(),
            true,
        );
        let count = self
            .gpu_cull
            .read_command_count(device, queue, enc, &mut self.indirect);
        self.stats.visible_meshlets = count;
        count
    }
}
