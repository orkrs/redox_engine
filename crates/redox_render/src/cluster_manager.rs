//! GPU cluster management and light assignment updates.

use crate::cluster_gpu_cull::{ClusterCullParams, ClusterGpuCull};
use crate::clustering::{self, ClusterInfo, ClusterLightAssignment};
use crate::light::{PointLight, PointLightGpu};
use wgpu::{Buffer, CommandEncoder, Device, Queue};

/// Manages cluster data and GPU buffers for clustered forward rendering.
pub struct ClusterManager {
    /// Current cluster information.
    pub cluster_info: ClusterInfo,
    /// Point lights storage buffer.
    pub point_lights_buffer: Buffer,
    /// Metadata buffer: (offset, count) for each cluster.
    pub metadata_buffer: Buffer,
    /// Light index buffer: flat array of light indices.
    pub light_indices_buffer: Buffer,
    /// Cluster bounds buffer (min_depth, max_depth for each cluster).
    pub bounds_buffer: Buffer,
    /// Cluster info uniform buffer.
    pub info_buffer: Buffer,
    /// Current light assignment (cached for debugging).
    current_assignment: Option<ClusterLightAssignment>,
    /// Cached cluster depth bounds (rebuilt only when camera range / resolution changes).
    cached_cluster_bounds: Vec<clustering::ClusterBounds>,
    bounds_cache_near: f32,
    bounds_cache_far: f32,
    bounds_cache_width: u32,
    bounds_cache_height: u32,
    /// When true, light→cluster assignment runs on GPU compute; CPU builds AABBs only.
    pub gpu_cull: bool,
    gpu_cull_pass: ClusterGpuCull,
    pending_gpu_lights: u32,
    pending_cam_pos: redox_math::Vec3,
    pending_view_proj: redox_math::Mat4,
}

/// Maximum number of light indices in the flat buffer (cap to avoid huge allocations).
const MAX_INDICES_CAP: u64 = 2 * 1024 * 1024;

impl ClusterManager {
    /// Creates a new cluster manager with initial screen and camera parameters.
    pub fn new(
        screen_width: u32,
        screen_height: u32,
        camera: &crate::camera::Camera,
        device: &Device,
        queue: &Queue,
    ) -> Self {
        let cluster_info = ClusterInfo::new(screen_width, screen_height, camera);
        let num_clusters = cluster_info.total_clusters();

        // Initialize point lights buffer
        let point_lights_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_point_lights_buffer"),
            size: (clustering::cluster_config::MAX_LIGHTS as u64)
                * std::mem::size_of::<PointLightGpu>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Initialize buffers with default sizes
        let metadata_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_metadata_buffer"),
            size: (num_clusters as u64) * std::mem::size_of::<clustering::ClusterMetadata>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Flat array of light indices per cluster; worst case num_clusters * MAX_LIGHTS_PER_CLUSTER.
        let indices_capacity =
            (num_clusters as u64) * (clustering::cluster_config::MAX_LIGHTS_PER_CLUSTER as u64);
        let light_indices_buffer_size =
            (indices_capacity.min(MAX_INDICES_CAP)) * std::mem::size_of::<u32>() as u64;
        let light_indices_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_light_indices_buffer"),
            size: light_indices_buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bounds_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_bounds_buffer"),
            size: (num_clusters as u64) * std::mem::size_of::<clustering::ClusterBounds>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let info_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_info_buffer"),
            size: std::mem::size_of::<ClusterInfo>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Write initial cluster info
        let info_bytes = bytemuck::bytes_of(&cluster_info);
        queue.write_buffer(&info_buffer, 0, info_bytes);

        Self {
            cluster_info,
            point_lights_buffer,
            metadata_buffer,
            light_indices_buffer,
            bounds_buffer,
            info_buffer,
            current_assignment: None,
            cached_cluster_bounds: Vec::new(),
            bounds_cache_near: f32::NAN,
            bounds_cache_far: f32::NAN,
            bounds_cache_width: 0,
            bounds_cache_height: 0,
            gpu_cull: true,
            gpu_cull_pass: ClusterGpuCull::new(device),
            pending_gpu_lights: 0,
            pending_cam_pos: redox_math::Vec3::ZERO,
            pending_view_proj: redox_math::Mat4::IDENTITY,
        }
    }

    fn invalidate_bounds_cache(&mut self) {
        self.cached_cluster_bounds.clear();
        self.bounds_cache_near = f32::NAN;
        self.bounds_cache_far = f32::NAN;
        self.bounds_cache_width = 0;
        self.bounds_cache_height = 0;
    }

    fn ensure_cluster_bounds(&mut self, camera: &crate::camera::Camera) {
        let w = self.cluster_info.screen_width;
        let h = self.cluster_info.screen_height;
        if self.cached_cluster_bounds.is_empty()
            || self.bounds_cache_near != camera.near
            || self.bounds_cache_far != camera.far
            || self.bounds_cache_width != w
            || self.bounds_cache_height != h
        {
            self.cached_cluster_bounds = clustering::build_clusters(camera, w, h);
            self.bounds_cache_near = camera.near;
            self.bounds_cache_far = camera.far;
            self.bounds_cache_width = w;
            self.bounds_cache_height = h;
        }
    }

    /// Rebuilds cluster buffers for a new screen resolution without requiring
    /// a full `Camera` struct (reuses the existing near/far values).
    pub fn resize_for_screen(
        &mut self,
        screen_width: u32,
        screen_height: u32,
        device: &Device,
        queue: &Queue,
    ) {
        use clustering::cluster_config;
        let new_clusters_x = screen_width.div_ceil(cluster_config::CLUSTER_SIZE_X);
        let new_clusters_y = screen_height.div_ceil(cluster_config::CLUSTER_SIZE_Y);

        let old_total = self.cluster_info.total_clusters();

        self.cluster_info.clusters_x = new_clusters_x;
        self.cluster_info.clusters_y = new_clusters_y;
        self.cluster_info.screen_width = screen_width;
        self.cluster_info.screen_height = screen_height;
        self.cluster_info.total_clusters =
            new_clusters_x * new_clusters_y * self.cluster_info.depth_slices;

        let new_total = self.cluster_info.total_clusters();

        if new_total > old_total {
            self.metadata_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster_metadata_buffer"),
                size: (new_total as u64)
                    * std::mem::size_of::<clustering::ClusterMetadata>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            self.bounds_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster_bounds_buffer"),
                size: (new_total as u64) * std::mem::size_of::<clustering::ClusterBounds>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let indices_cap =
                (new_total as u64) * (clustering::cluster_config::MAX_LIGHTS_PER_CLUSTER as u64);
            let indices_size =
                (indices_cap.min(MAX_INDICES_CAP)) * std::mem::size_of::<u32>() as u64;
            self.light_indices_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster_light_indices_buffer"),
                size: indices_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        queue.write_buffer(&self.info_buffer, 0, bytemuck::bytes_of(&self.cluster_info));
        self.current_assignment = None;
        self.invalidate_bounds_cache();
    }
    /// Rebuilds screen dimensions (e.g., on window resize).
    pub fn rebuild_for_resolution(
        &mut self,
        screen_width: u32,
        screen_height: u32,
        camera: &crate::camera::Camera,
        device: &Device,
        queue: &Queue,
    ) {
        let new_info = ClusterInfo::new(screen_width, screen_height, camera);
        let old_total = self.cluster_info.total_clusters();
        let new_total = new_info.total_clusters();

        // Recreate buffers if size changed
        if new_total != old_total {
            self.metadata_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster_metadata_buffer"),
                size: (new_total as u64)
                    * std::mem::size_of::<clustering::ClusterMetadata>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            self.bounds_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster_bounds_buffer"),
                size: (new_total as u64) * std::mem::size_of::<clustering::ClusterBounds>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let indices_cap =
                (new_total as u64) * (clustering::cluster_config::MAX_LIGHTS_PER_CLUSTER as u64);
            let indices_size =
                (indices_cap.min(MAX_INDICES_CAP)) * std::mem::size_of::<u32>() as u64;
            self.light_indices_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cluster_light_indices_buffer"),
                size: indices_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        self.cluster_info = new_info;

        // Write updated cluster info
        let info_bytes = bytemuck::bytes_of(&self.cluster_info);
        queue.write_buffer(&self.info_buffer, 0, info_bytes);

        self.current_assignment = None;
        self.invalidate_bounds_cache();
    }

    /// Updates cluster light assignments based on current lights.
    ///
    /// `cam_pos` must be the camera's world-space position (same value written into
    /// `camera_uniform.camera_pos`) so the CPU depth test matches the WGSL shader.
    pub fn update_clusters(
        &mut self,
        lights: &[PointLight],
        camera: &crate::camera::Camera,
        cam_pos: redox_math::Vec3,
        view_proj: redox_math::Mat4,
        _device: &Device,
        queue: &Queue,
    ) {
        // Convert PointLight to PointLightGpu and serialize to GPU
        let gpu_lights: Vec<PointLightGpu> =
            lights.iter().map(PointLightGpu::from_point_light).collect();

        if !gpu_lights.is_empty() {
            let lights_bytes = bytemuck::cast_slice::<PointLightGpu, u8>(&gpu_lights);
            queue.write_buffer(&self.point_lights_buffer, 0, lights_bytes);
        }

        self.ensure_cluster_bounds(camera);

        let bounds_bytes =
            bytemuck::cast_slice::<clustering::ClusterBounds, u8>(&self.cached_cluster_bounds);
        queue.write_buffer(&self.bounds_buffer, 0, bounds_bytes);

        if self.gpu_cull {
            self.pending_gpu_lights = gpu_lights.len() as u32;
            self.pending_cam_pos = cam_pos;
            self.pending_view_proj = view_proj;
            self.current_assignment = None;
            return;
        }

        let assignment = clustering::assign_lights_to_clusters(
            lights,
            &self.cluster_info,
            &self.cached_cluster_bounds,
            cam_pos,
            view_proj,
        );

        let metadata_bytes =
            bytemuck::cast_slice::<clustering::ClusterMetadata, u8>(&assignment.cluster_metadata);
        queue.write_buffer(&self.metadata_buffer, 0, metadata_bytes);

        let indices_bytes = bytemuck::cast_slice::<u32, u8>(&assignment.light_indices);
        queue.write_buffer(&self.light_indices_buffer, 0, indices_bytes);

        self.current_assignment = Some(assignment);
    }

    /// Records the GPU cluster light cull into `encoder` (no-op when [`Self::gpu_cull`] is false).
    pub fn record_gpu_cull(&self, device: &Device, encoder: &mut CommandEncoder, queue: &Queue) {
        if !self.gpu_cull {
            return;
        }

        let indices_capacity = (self.light_indices_buffer.size()
            / std::mem::size_of::<u32>() as u64)
            .min(u32::MAX as u64) as u32;

        let params = ClusterCullParams {
            cam_pos: [
                self.pending_cam_pos.x,
                self.pending_cam_pos.y,
                self.pending_cam_pos.z,
                1.0,
            ],
            view_proj: self.pending_view_proj.to_cols_array_2d(),
            num_lights: self.pending_gpu_lights,
            max_lights_per_cluster: clustering::cluster_config::MAX_LIGHTS_PER_CLUSTER,
            indices_capacity,
            _pad: 0,
        };

        self.gpu_cull_pass.record_dispatch(
            device,
            encoder,
            queue,
            &params,
            &self.info_buffer,
            &self.bounds_buffer,
            &self.point_lights_buffer,
            &self.metadata_buffer,
            &self.light_indices_buffer,
            self.cluster_info.total_clusters,
        );
    }

    /// Returns the current cluster info.
    pub fn info(&self) -> &ClusterInfo {
        &self.cluster_info
    }

    /// Returns statistics about the current light assignment (for debugging).
    pub fn assignment_stats(&self) -> Option<ClusterAssignmentStats> {
        self.current_assignment.as_ref().map(|assignment| {
            let total_lights_refs = assignment.light_indices.len() as u32;
            let total_clusters = assignment.cluster_count;
            let avg_lights_per_cluster = if total_clusters > 0 {
                total_lights_refs as f32 / total_clusters as f32
            } else {
                0.0
            };

            let max_lights = assignment
                .cluster_metadata
                .iter()
                .map(|m| m.count)
                .max()
                .unwrap_or(0);

            ClusterAssignmentStats {
                total_lights_in_scene: assignment.cluster_metadata.iter().map(|m| m.count).sum(),
                total_light_references: total_lights_refs,
                total_clusters,
                avg_lights_per_cluster,
                max_lights_in_cluster: max_lights,
            }
        })
    }
}

/// Statistics about the current light-to-cluster assignment.
#[derive(Clone, Debug)]
pub struct ClusterAssignmentStats {
    /// Number of unique lights in the scene.
    pub total_lights_in_scene: u32,
    /// Total number of light references across all clusters (sum of all counts).
    pub total_light_references: u32,
    /// Total number of clusters.
    pub total_clusters: u32,
    /// Average lights per cluster.
    pub avg_lights_per_cluster: f32,
    /// Maximum lights in any single cluster.
    pub max_lights_in_cluster: u32,
}
