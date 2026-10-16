// GPU clustered light assignment — one invocation per cluster.
// Cluster AABBs (depth bands) are built on CPU and uploaded to `cluster_bounds`.
// Depth metric is view-space Z (planar), matching fragment shaders.

struct ClusterBounds {
    min_depth: f32,
    max_depth: f32,
}

struct ClusterMetadata {
    offset: u32,
    count: u32,
}

struct ClusterInfo {
    clusters_x: u32,
    clusters_y: u32,
    depth_slices: u32,
    screen_width: u32,
    screen_height: u32,
    near: f32,
    far: f32,
    depth_scale: f32,
    total_clusters: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct PointLight {
    position: vec4<f32>,
    color: vec4<f32>,
    shadow_matrices: array<mat4x4<f32>, 6>,
    intensity: f32,
    radius: f32,
    shadow_tile_base: u32,
    _pad: f32,
}

struct CullParams {
    cam_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    num_lights: u32,
    max_lights_per_cluster: u32,
    indices_capacity: u32,
    _pad0: u32,
}

const MAX_LOCAL_LIGHTS: u32 = 256u;

@group(0) @binding(0) var<uniform> cluster_info: ClusterInfo;
@group(0) @binding(1) var<uniform> cull_params: CullParams;
@group(0) @binding(2) var<storage, read> cluster_bounds: array<ClusterBounds>;
@group(0) @binding(3) var<storage, read> point_lights: array<PointLight>;
@group(0) @binding(4) var<storage, read_write> cluster_metadata: array<ClusterMetadata>;
@group(0) @binding(5) var<storage, read_write> light_indices: array<u32>;
@group(0) @binding(6) var<storage, read_write> global_light_offset: atomic<u32>;

fn sphere_intersects_cluster(
    light_pos: vec3<f32>,
    light_radius: f32,
    cluster_idx: u32,
) -> bool {
    let bounds = cluster_bounds[cluster_idx];

    // Perspective: clip.w == view-space depth along -Z (planar), not euclidean.
    let clip = cull_params.view_proj * vec4<f32>(light_pos, 1.0);
    let light_view_z = clip.w;

    if (light_view_z <= 0.0 && light_view_z + light_radius <= 0.0) {
        return false;
    }

    let sphere_near = max(light_view_z - light_radius, 0.0);
    let sphere_far = light_view_z + light_radius;
    if (!(sphere_near <= bounds.max_depth && sphere_far >= bounds.min_depth)) {
        return false;
    }

    // Behind / straddling near plane: skip bogus NDC (matches CPU cull).
    if (clip.w <= 0.0) {
        return false;
    }
    let w = clip.w;
    let ndc = clip.xyz / w;

    let tile_x = cluster_idx % cluster_info.clusters_x;
    let tile_y = (cluster_idx / cluster_info.clusters_x) % cluster_info.clusters_y;
    let tile_min_x = (f32(tile_x) / f32(cluster_info.clusters_x)) * 2.0 - 1.0;
    let tile_max_x = (f32(tile_x + 1u) / f32(cluster_info.clusters_x)) * 2.0 - 1.0;
    let tile_max_y = 1.0 - (f32(tile_y) / f32(cluster_info.clusters_y)) * 2.0;
    let tile_min_y = 1.0 - (f32(tile_y + 1u) / f32(cluster_info.clusters_y)) * 2.0;

    // Row-wise scale (matches clustering.rs): m[col].row components.
    let scale_x = max(
        max(abs(cull_params.view_proj[0].x), abs(cull_params.view_proj[1].x)),
        abs(cull_params.view_proj[2].x),
    );
    let scale_y = max(
        max(abs(cull_params.view_proj[0].y), abs(cull_params.view_proj[1].y)),
        abs(cull_params.view_proj[2].y),
    );
    let r_ndc_x = light_radius * scale_x / w;
    let r_ndc_y = light_radius * scale_y / w;
    if (ndc.x + r_ndc_x < tile_min_x || ndc.x - r_ndc_x > tile_max_x) {
        return false;
    }
    if (ndc.y + r_ndc_y < tile_min_y || ndc.y - r_ndc_y > tile_max_y) {
        return false;
    }
    return true;
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cluster_idx = gid.x;
    if (cluster_idx >= cluster_info.total_clusters) {
        return;
    }

    var count = 0u;
    var local: array<u32, MAX_LOCAL_LIGHTS>;

    let num_lights = cull_params.num_lights;
    let cap = cull_params.max_lights_per_cluster;
    for (var li = 0u; li < num_lights; li = li + 1u) {
        let light = point_lights[li];
        if (sphere_intersects_cluster(light.position.xyz, light.radius, cluster_idx)) {
            if (count < cap) {
                local[count] = li;
                count = count + 1u;
            }
        }
    }

    var write_count = count;
    let offset = atomicAdd(&global_light_offset, write_count);
    // Clamp against compacted index buffer capacity (may be < worst-case).
    let cap_idx = cull_params.indices_capacity;
    if (offset >= cap_idx) {
        write_count = 0u;
    } else if (offset + write_count > cap_idx) {
        write_count = cap_idx - offset;
    }
    cluster_metadata[cluster_idx] = ClusterMetadata(offset, write_count);
    for (var i = 0u; i < write_count; i = i + 1u) {
        light_indices[offset + i] = local[i];
    }
}
