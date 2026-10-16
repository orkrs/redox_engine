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
    /// Total number of clusters in the 3D grid (clusters_x * clusters_y * depth_slices).
    pub total_clusters: u32,
    /// WGSL uniform structs round size to 16 bytes.
    pub _pad: [u32; 3],
}

impl ClusterInfo {
    /// Builds cluster info from screen and camera parameters.
    pub fn new(screen_width: u32, screen_height: u32, camera: &Camera) -> Self {
        let clusters_x = screen_width.div_ceil(cluster_config::CLUSTER_SIZE_X);
        let clusters_y = screen_height.div_ceil(cluster_config::CLUSTER_SIZE_Y);
        let depth_slices = cluster_config::CLUSTER_DEPTH;
        let total_clusters = clusters_x * clusters_y * depth_slices;

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
            total_clusters,
            _pad: [0; 3],
        }
    }

    /// Total number of clusters (convenience for usize-based code).
    pub fn total_clusters(&self) -> usize {
        // Derive from dimensions to avoid stale cached values after resize.
        (self.clusters_x as usize) * (self.clusters_y as usize) * (self.depth_slices as usize)
    }
}

/// Builds the cluster bounds for the given camera and screen dimensions.
pub fn build_clusters(
    camera: &Camera,
    screen_width: u32,
    screen_height: u32,
) -> Vec<ClusterBounds> {
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

/// Linearizes a hardware depth value (NDC z, **[0, 1]** range as produced by
/// [`redox_math::perspective`] on wgpu) into positive view-space distance.
pub fn linearize_depth(depth_ndc: f32, near: f32, far: f32) -> f32 {
    // Zero-to-one perspective: d = (-f/(f-n) * z - n*f/(f-n)) / z
    // solved for |z|:
    //   z = n*f / (f - d*(f-n))
    let denom = far - depth_ndc * (far - near);
    if denom.abs() < 1e-6 {
        return far;
    }
    near * far / denom
}

/// Converts linear depth to a depth slice index.
pub fn get_depth_slice(linear_depth: f32, cluster_info: &ClusterInfo) -> u32 {
    let near = cluster_info.near;
    let _depth_scale = cluster_info.depth_scale;

    // z_slice = ln(depth / near) / depth_scale
    let z_norm = ((linear_depth / near).max(1.0).ln() / _depth_scale).max(0.0);
    (z_norm as u32).min(cluster_info.depth_slices - 1)
}
/// Returns true if a point light sphere could illuminate any fragment in the given cluster.
///
/// Two-stage conservative test:
/// 1. **Depth** — the light's near/far extent along the camera→light ray must
///    overlap the cluster's logarithmic depth band (matches the WGSL metric).
/// 2. **Screen-space XY** — project the light center to NDC and test a
///    conservatively enlarged sphere footprint against the cluster's screen
///    rectangle. Lights outside the frustum (or behind the camera) are only
///    rejected when provably off-screen; borderline cases pass.
fn sphere_intersects_cluster(
    light_pos: Vec3,
    light_radius: f32,
    cluster_x: u32,
    cluster_y: u32,
    cluster_z: u32,
    cluster_info: &ClusterInfo,
    cluster_bounds: &[ClusterBounds],
    _cam_pos: Vec3,
    view_proj: redox_math::Mat4,
) -> bool {
    let cluster_idx = (cluster_z as usize)
        * (cluster_info.clusters_x as usize)
        * (cluster_info.clusters_y as usize)
        + (cluster_y as usize) * (cluster_info.clusters_x as usize)
        + (cluster_x as usize);

    if cluster_idx >= cluster_bounds.len() {
        return false;
    }

    let bounds = &cluster_bounds[cluster_idx];

    // ── 1. Depth overlap — view-space Z from clip.w (planar, matches shaders) ──
    let clip_depth = view_proj * light_pos.extend(1.0);
    let light_view_z = clip_depth.w;
    if light_view_z <= 0.0 && light_view_z + light_radius <= 0.0 {
        return false;
    }
    let sphere_near = (light_view_z - light_radius).max(0.0);
    let sphere_far = light_view_z + light_radius;
    if !(sphere_near <= bounds.max_depth && sphere_far >= bounds.min_depth) {
        return false;
    }

    // ── 2. Screen-space XY overlap against THIS cluster's tile ──
    let clip = clip_depth;
    let w = clip.w;
    if w <= 0.0 {
        return false;
    }
    let ndc = clip.truncate() / w;

    let tile_min_x = (cluster_x as f32 / cluster_info.clusters_x as f32) * 2.0 - 1.0;
    let tile_max_x = ((cluster_x + 1) as f32 / cluster_info.clusters_x as f32) * 2.0 - 1.0;
    let tile_max_y = 1.0 - (cluster_y as f32 / cluster_info.clusters_y as f32) * 2.0;
    let tile_min_y = 1.0 - ((cluster_y + 1) as f32 / cluster_info.clusters_y as f32) * 2.0;

    let m = view_proj.to_cols_array_2d();
    let scale_x = m[0][0].abs().max(m[1][0].abs()).max(m[2][0].abs());
    let scale_y = m[0][1].abs().max(m[1][1].abs()).max(m[2][1].abs());
    let r_ndc_x = light_radius * scale_x / w;
    let r_ndc_y = light_radius * scale_y / w;
    if ndc.x + r_ndc_x < tile_min_x || ndc.x - r_ndc_x > tile_max_x {
        return false;
    }
    if ndc.y + r_ndc_y < tile_min_y || ndc.y - r_ndc_y > tile_max_y {
        return false;
    }

    true
}

/// Assigns lights to clusters based on spatial intersection.
///
/// `cam_pos` is the camera's world-space position; it must match what
/// `camera_uniform.camera_pos` contains so the depth metric aligns with the shader.
///
/// Returns indices into the lights array for each cluster, along with metadata.
pub fn assign_lights_to_clusters(
    lights: &[PointLight],
    cluster_info: &ClusterInfo,
    cluster_bounds: &[ClusterBounds],
    cam_pos: Vec3,
    view_proj: redox_math::Mat4,
) -> ClusterLightAssignment {
    // Derive this from dimensions directly to avoid any mismatch with the cached
    // `total_clusters` field (and to stay safe across resizes).
    let num_clusters = (cluster_info.clusters_x as usize)
        * (cluster_info.clusters_y as usize)
        * (cluster_info.depth_slices as usize);
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
                let cluster_idx = (cluster_z as usize)
                    * (cluster_info.clusters_x as usize)
                    * (cluster_info.clusters_y as usize)
                    + (cluster_y as usize) * (cluster_info.clusters_x as usize)
                    + (cluster_x as usize);
                if cluster_idx >= cluster_metadata.len() {
                    // Defensive guard: should never happen, but avoids a hard crash.
                    continue;
                }

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
                        cam_pos,
                        view_proj,
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

        assert_eq!(info.clusters_x, 1920_u32.div_ceil(16));
        assert_eq!(info.clusters_y, 1080_u32.div_ceil(16));
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
