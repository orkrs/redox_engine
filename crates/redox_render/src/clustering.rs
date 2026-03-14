//! Clustered forward rendering: spatial partitioning of lights into screen-space clusters.
//!
//! This module implements a 3D grid of clusters in screen space and depth, allowing efficient
//! light culling. Each cluster stores which lights affect it, reducing the per-fragment light loop.

use crate::camera::Camera;
use crate::light::PointLight;
use redox_math::Vec3;

/// Constants for cluster grid configuration.
pub mod cluster_config {
    /// Size of each cluster in screen-space X (pixels).
    pub const CLUSTER_SIZE_X: u32 = 16;
    /// Size of each cluster in screen-space Y (pixels).
    pub const CLUSTER_SIZE_Y: u32 = 16;
    /// Number of depth slices in the cluster grid.
    pub const CLUSTER_DEPTH: u32 = 24;
    /// Maximum number of lights per cluster.
    pub const MAX_LIGHTS_PER_CLUSTER: u32 = 256;
    /// Maximum total lights in the scene.
    pub const MAX_LIGHTS: u32 = 512;
}

/// Stores depth bounds for a single cluster.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ClusterBounds {
    /// Minimum depth in camera space (linear, typically near_plane).
    pub min_depth: f32,
    /// Maximum depth in camera space (linear).
    pub max_depth: f32,
}

/// Metadata for a cluster's light list: offset and count.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ClusterMetadata {
    /// Offset into the global light index array.
    pub offset: u32,
    /// Number of lights in this cluster.
    pub count: u32,
}

/// Clustered light assignment result.
pub struct ClusterLightAssignment {
    /// Flat array of light indices.
    pub light_indices: Vec<u32>,
    /// Metadata (offset and count) for each cluster.
    pub cluster_metadata: Vec<ClusterMetadata>,
    /// Cluster bounds (min/max depth) for each cluster.
    pub cluster_bounds: Vec<ClusterBounds>,
    /// Actual number of clusters used.
    pub cluster_count: u32,
    /// Actual number of depth slices used (may be less than CLUSTER_DEPTH).
    pub depth_slices: u32,
}

/// Information about the cluster grid (for GPU).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ClusterInfo {
    /// Number of clusters in X (width / CLUSTER_SIZE_X).
    pub clusters_x: u32,
    /// Number of clusters in Y (height / CLUSTER_SIZE_Y).
    pub clusters_y: u32,
    /// Number of depth slices.
    pub depth_slices: u32,
    /// Screen width in pixels.
    pub screen_width: u32,
    /// Screen height in pixels.
    pub screen_height: u32,
    /// Camera near plane.
    pub near: f32,
    /// Camera far plane.
    pub far: f32,
    /// Log-based depth scaling factor (for logarithmic depth distribution).
    pub depth_scale: f32,
}

impl ClusterInfo {
    /// Builds cluster info from screen and camera parameters.
    pub fn new(screen_width: u32, screen_height: u32, camera: &Camera) -> Self {
        let clusters_x = (screen_width + cluster_config::CLUSTER_SIZE_X - 1) / cluster_config::CLUSTER_SIZE_X;
        let clusters_y = (screen_height + cluster_config::CLUSTER_SIZE_Y - 1) / cluster_config::CLUSTER_SIZE_Y;
        let depth_slices = cluster_config::CLUSTER_DEPTH;

        // Logarithmic depth scaling: z_slice = near * (far/near)^(i/(num_slices-1))
        // This allows better precision near the camera.
        let depth_scale = (camera.far / camera.near).ln() / (depth_slices - 1) as f32;

        Self {
            clusters_x,
            clusters_y,
            depth_slices,
            screen_width,
            screen_height,
            near: camera.near,
            far: camera.far,
            depth_scale,
        }
    }

    /// Total number of clusters.
    pub fn total_clusters(&self) -> usize {
        (self.clusters_x as usize) * (self.clusters_y as usize) * (self.depth_slices as usize)
    }
}

/// Builds the cluster bounds for the given camera and screen dimensions.
pub fn build_clusters(camera: &Camera, screen_width: u32, screen_height: u32) -> Vec<ClusterBounds> {
    let cluster_info = ClusterInfo::new(screen_width, screen_height, camera);
    let mut bounds = Vec::with_capacity(cluster_info.total_clusters());

    let near = camera.near;
    let far = camera.far;

    for z in 0..cluster_info.depth_slices {
        // Logarithmic depth distribution: z_i = near * (far/near)^(i / (num_slices - 1))
        // This gives better precision near the camera.
        let z_norm = z as f32 / (cluster_info.depth_slices - 1) as f32;
        let min_depth = near * (far / near).powf(z_norm);
        let max_depth = if z == cluster_info.depth_slices - 1 {
            far
        } else {
            let z_norm_next = (z as f32 + 1.0) / (cluster_info.depth_slices - 1) as f32;
            near * (far / near).powf(z_norm_next)
        };

        for _y in 0..cluster_info.clusters_y {
            for _x in 0..cluster_info.clusters_x {
                bounds.push(ClusterBounds {
                    min_depth,
                    max_depth,
                });
            }
        }
    }

    bounds
}

/// Computes the cluster index from screen coordinates and depth.
///
/// Returns (cluster_x, cluster_y, cluster_z) or None if out of bounds.
pub fn get_cluster_indices(
    screen_x: f32,
    screen_y: f32,
    linear_depth: f32,
    cluster_info: &ClusterInfo,
) -> Option<(u32, u32, u32)> {
    let cluster_x = (screen_x / cluster_config::CLUSTER_SIZE_X as f32) as u32;
    let cluster_y = (screen_y / cluster_config::CLUSTER_SIZE_Y as f32) as u32;

    if cluster_x >= cluster_info.clusters_x || cluster_y >= cluster_info.clusters_y {
        return None;
    }

    // Logarithmic depth indexing
    let z_norm = ((linear_depth / cluster_info.near).ln() / cluster_info.depth_scale).max(0.0);
    let cluster_z = (z_norm as u32).min(cluster_info.depth_slices - 1);

    Some((cluster_x, cluster_y, cluster_z))
}

/// Linearizes depth from NDC (normalized device coordinates).
/// 
/// Given a depth value from the depth buffer (typically in [0, 1] in NDC),
/// this recovers the linear depth in camera space.
pub fn linearize_depth(depth_ndc: f32, near: f32, far: f32) -> f32 {
    // Assuming standard perspective projection:
    // depth_ndc = (near + far) / (far - near) + (2 * near * far) / ((far - near) * depth_linear)
    // Solve for depth_linear:
    let depth_linear = (2.0 * near * far) / ((far + near) - depth_ndc * (far - near));
    depth_linear
}

/// Converts linear depth to a depth slice index.
pub fn get_depth_slice(linear_depth: f32, cluster_info: &ClusterInfo) -> u32 {
    let near = cluster_info.near;
    let _depth_scale = cluster_info.depth_scale;
    
    // z_slice = ln(depth / near) / depth_scale
    let z_norm = ((linear_depth / near).max(1.0).ln() / _depth_scale).max(0.0);
    (z_norm as u32).min(cluster_info.depth_slices - 1)
}

/// Checks if a sphere (light) intersects with a cluster in screen-space and depth.
///
/// This is a conservative test: it checks if the light's bounding sphere could intersect
/// the cluster's bounds. A more precise implementation could use frustum intersection.
fn sphere_intersects_cluster(
    light_pos: Vec3,
    light_radius: f32,
    cluster_x: u32,
    cluster_y: u32,
    cluster_z: u32,
    cluster_info: &ClusterInfo,
    cluster_bounds: &[ClusterBounds],
) -> bool {
    // Check depth intersection
    let cluster_idx = (cluster_z as usize) * (cluster_info.clusters_x as usize)
        * (cluster_info.clusters_y as usize)
        + (cluster_y as usize) * (cluster_info.clusters_x as usize)
        + (cluster_x as usize);

    if cluster_idx >= cluster_bounds.len() {
        return false;
    }

    let bounds = &cluster_bounds[cluster_idx];
    let depth_min = bounds.min_depth;
    let depth_max = bounds.max_depth;

    // Conservative depth check: light must be within [depth_min - radius, depth_max + radius]
    if light_pos.z > depth_max + light_radius || light_pos.z + light_radius < depth_min {
        return false;
    }

    // Screen-space AABB check (simplified; assumes light at given depth)
    // For a point light, we compute its approximate screen-space radius
    let screen_radius = (light_radius / light_pos.z).max(0.1) * 1000.0; // Rough projection
    
    let cluster_min_x = (cluster_x as f32) * cluster_config::CLUSTER_SIZE_X as f32;
    let cluster_max_x = cluster_min_x + cluster_config::CLUSTER_SIZE_X as f32;
    let cluster_min_y = (cluster_y as f32) * cluster_config::CLUSTER_SIZE_Y as f32;
    let cluster_max_y = cluster_min_y + cluster_config::CLUSTER_SIZE_Y as f32;

    // Rough circle-rect intersection: does the light's screen-space circle intersect the cluster rect?
    let dx = light_pos.x.max(cluster_min_x).min(cluster_max_x) - light_pos.x;
    let dy = light_pos.y.max(cluster_min_y).min(cluster_max_y) - light_pos.y;
    let dist_sq = dx * dx + dy * dy;

    dist_sq <= screen_radius * screen_radius
}

/// Assigns lights to clusters based on spatial intersection.
///
/// Returns indices into the lights array for each cluster, along with metadata.
pub fn assign_lights_to_clusters(
    lights: &[PointLight],
    cluster_info: &ClusterInfo,
    cluster_bounds: &[ClusterBounds],
) -> ClusterLightAssignment {
    let num_clusters = cluster_info.total_clusters();
    let mut cluster_metadata: Vec<ClusterMetadata> = vec![
        ClusterMetadata {
            offset: 0,
            count: 0,
        };
        num_clusters
    ];
    let mut light_indices = Vec::new();

    // For each cluster, collect intersecting lights
    let mut current_offset = 0u32;
    for cluster_z in 0..cluster_info.depth_slices {
        for cluster_y in 0..cluster_info.clusters_y {
            for cluster_x in 0..cluster_info.clusters_x {
                let cluster_idx = (cluster_z as usize) * (cluster_info.clusters_x as usize)
                    * (cluster_info.clusters_y as usize)
                    + (cluster_y as usize) * (cluster_info.clusters_x as usize)
                    + (cluster_x as usize);

                let mut light_count = 0u32;
                for (light_idx, light) in lights.iter().enumerate() {
                    if sphere_intersects_cluster(
                        light.position,
                        light.radius,
                        cluster_x,
                        cluster_y,
                        cluster_z,
                        cluster_info,
                        cluster_bounds,
                    ) {
                        light_indices.push(light_idx as u32);
                        light_count += 1;

                        // Cap lights per cluster to prevent excessive computation
                        if light_count >= cluster_config::MAX_LIGHTS_PER_CLUSTER {
                            break;
                        }
                    }
                }

                cluster_metadata[cluster_idx] = ClusterMetadata {
                    offset: current_offset,
                    count: light_count,
                };
                current_offset += light_count;
            }
        }
    }

    ClusterLightAssignment {
        light_indices,
        cluster_metadata,
        cluster_bounds: cluster_bounds.to_vec(),
        cluster_count: num_clusters as u32,
        depth_slices: cluster_info.depth_slices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_info_computation() {
        let camera = Camera::new(std::f32::consts::PI / 4.0, 16.0 / 9.0, 0.1, 1000.0);
        let info = ClusterInfo::new(1920, 1080, &camera);

        assert_eq!(info.clusters_x, (1920 + 15) / 16);
        assert_eq!(info.clusters_y, (1080 + 15) / 16);
        assert_eq!(info.depth_slices, cluster_config::CLUSTER_DEPTH);
        assert_eq!(info.screen_width, 1920);
        assert_eq!(info.screen_height, 1080);
    }

    #[test]
    fn test_get_cluster_indices() {
        let camera = Camera::new(std::f32::consts::PI / 4.0, 16.0 / 9.0, 0.1, 1000.0);
        let info = ClusterInfo::new(1920, 1080, &camera);

        // Test valid cluster
        let (cx, cy, cz) = get_cluster_indices(100.0, 200.0, 5.0, &info).unwrap();
        assert_eq!(cx, 100 / 16);
        assert_eq!(cy, 200 / 16);
        assert!(cz < info.depth_slices);

        // Test out of bounds
        let result = get_cluster_indices(2000.0, 200.0, 5.0, &info);
        assert!(result.is_none());
    }
}
