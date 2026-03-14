//! GPU cluster management and light assignment updates.

use crate::clustering::{self, ClusterInfo, ClusterLightAssignment};
use crate::light::{PointLight, PointLightGpu};
use wgpu::{Buffer, Device, Queue};

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
}

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

        let light_indices_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_light_indices_buffer"),
            size: (clustering::cluster_config::MAX_LIGHTS as u64)
                * std::mem::size_of::<u32>() as u64,
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
        }
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
        }

        self.cluster_info = new_info;
        
        // Write updated cluster info
        let info_bytes = bytemuck::bytes_of(&self.cluster_info);
        queue.write_buffer(&self.info_buffer, 0, info_bytes);

        self.current_assignment = None;
    }

    /// Updates cluster light assignments based on current lights.
    pub fn update_clusters(
        &mut self,
        lights: &[PointLight],
        camera: &crate::camera::Camera,
        _device: &Device,
        queue: &Queue,
    ) {
        // Convert PointLight to PointLightGpu and serialize to GPU
        let gpu_lights: Vec<PointLightGpu> = lights
            .iter()
            .map(PointLightGpu::from_point_light)
            .collect();
        
        if !gpu_lights.is_empty() {
            let lights_bytes = bytemuck::cast_slice::<PointLightGpu, u8>(&gpu_lights);
            queue.write_buffer(&self.point_lights_buffer, 0, lights_bytes);
        }

        // Build cluster bounds
        let cluster_bounds = clustering::build_clusters(
            camera,
            self.cluster_info.screen_width,
            self.cluster_info.screen_height,
        );

        // Assign lights to clusters
        let assignment = clustering::assign_lights_to_clusters(lights, &self.cluster_info, &cluster_bounds);

        // Write metadata buffer
        let metadata_bytes = bytemuck::cast_slice::<clustering::ClusterMetadata, u8>(&assignment.cluster_metadata);
        queue.write_buffer(&self.metadata_buffer, 0, metadata_bytes);

        // Write light indices buffer
        let indices_bytes = bytemuck::cast_slice::<u32, u8>(&assignment.light_indices);
        queue.write_buffer(&self.light_indices_buffer, 0, indices_bytes);

        // Write bounds buffer
        let bounds_bytes = bytemuck::cast_slice::<clustering::ClusterBounds, u8>(&assignment.cluster_bounds);
        queue.write_buffer(&self.bounds_buffer, 0, bounds_bytes);

        self.current_assignment = Some(assignment);
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
