//! Built-in WGSL shaders and helper to create shader modules.
//!
//! In the MVP the shader source is embedded as a constant string.
//! This will be replaced by file-based loading in later milestones.

/// WGSL source for the default forward-shading shader.
///
/// ## Bind groups
/// - Group 0, Binding 0: `CameraUniform` (view_proj + camera_pos).
/// - Group 1, Binding 0: `LightUniform`  (direction, colour, ambient).
/// - Group 2, Binding 0: `ModelUniform`  (model matrix).
/// - Group 3, Binding 0: Texture.
/// - Group 3, Binding 1: Sampler.
pub const FORWARD_SHADER_SRC: &str = r#"
// ---- Uniforms ----

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct LightUniform {
    dir_color:      vec4<f32>,
    dir_direction:  vec4<f32>,
    ambient:        vec4<f32>,
    point_lights_pos:   array<vec4<f32>, 128>,
    point_lights_color: array<vec4<f32>, 128>,
    num_point_lights:   u32,
    pad0:               u32,
    pad1:               u32,
    pad2:               u32,
};

struct ModelUniform {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> light_u: LightUniform;
@group(2) @binding(0) var<storage, read> models: array<ModelUniform>;
@group(3) @binding(0) var t_diffuse: texture_2d<f32>;
@group(3) @binding(1) var s_diffuse: sampler;

// ---- Vertex stage ----

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) uv:       vec2<f32>,
    @location(3) tangent:  vec3<f32>,
    @builtin(instance_index) instance_index: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv:           vec2<f32>,
    @location(2) world_pos:    vec3<f32>,
    @location(3) instance_color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let model_data = models[in.instance_index];
    let world_pos = model_data.model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.world_normal = normalize((model_data.model * vec4<f32>(in.normal, 0.0)).xyz);
    out.uv = in.uv;
    out.world_pos = world_pos.xyz;
    out.instance_color = model_data.color;
    return out;
}

// ---- Fragment stage ----

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_diffuse, s_diffuse, in.uv);
    let n = normalize(in.world_normal);

    var total_diffuse = vec3<f32>(0.0);

    // Directional light
    let l_dir = normalize(light_u.dir_direction.xyz);
    let diff_dir = max(dot(n, l_dir), 0.0);
    total_diffuse += light_u.dir_color.xyz * diff_dir;

    // Point lights
    for (var i = 0u; i < light_u.num_point_lights; i = i + 1u) {
        let p_pos = light_u.point_lights_pos[i].xyz;
        let p_intensity = light_u.point_lights_pos[i].w;
        let p_color = light_u.point_lights_color[i].xyz;
        let p_radius = light_u.point_lights_color[i].w;

        let l_vec = p_pos - in.world_pos;
        let dist = length(l_vec);
        let l_p = normalize(l_vec);

        let diff_p = max(dot(n, l_p), 0.0);
        
        // Simple attenuation
        let attenuation = p_intensity * (1.0 - smoothstep(0.0, p_radius, dist));
        total_diffuse += p_color * diff_p * attenuation;
    }

    let final_color = (light_u.ambient.xyz + total_diffuse) * tex_color.xyz * in.instance_color.xyz;
    return vec4<f32>(final_color, tex_color.a * in.instance_color.a);
}
"#;

pub const PBR_SHADER_SRC: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct LightUniform {
    dir_color:      vec4<f32>,
    dir_direction:  vec4<f32>,
    ambient:        vec4<f32>,
    shadow_view_proj: mat4x4<f32>,
    point_lights_pos:   array<vec4<f32>, 128>,
    point_lights_color: array<vec4<f32>, 128>,
    num_point_lights:   u32,
    _pad0:              u32,
    _pad1:              u32,
    _pad2:              u32,
};

struct Material {
    base_color: vec4<f32>,
    emissive:   vec4<f32>,
    metallic:   f32,
    roughness:  f32,
    flags:      u32,
};

struct Model {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> light_u: LightUniform;
@group(0) @binding(2) var t_shadow: texture_depth_2d;
@group(0) @binding(3) var s_shadow: sampler_comparison;
@group(0) @binding(4) var t_irradiance: texture_cube<f32>;
@group(0) @binding(5) var t_prefiltered: texture_cube<f32>;
@group(0) @binding(6) var t_brdf_lut: texture_2d<f32>;
@group(0) @binding(7) var t_ssao: texture_2d<f32>;
@group(0) @binding(8) var s_ssao: sampler;
@group(0) @binding(9) var s_ibl: sampler;
// Clustered Forward Rendering
@group(0) @binding(10) var<storage, read> point_lights: array<PointLight>;
@group(0) @binding(11) var<storage, read> cluster_metadata: array<ClusterMetadata>;
@group(0) @binding(12) var<storage, read> cluster_light_indices: array<u32>;
@group(0) @binding(13) var<uniform> cluster_info: ClusterInfo;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var t_albedo: texture_2d<f32>;
@group(1) @binding(2) var t_normal: texture_2d<f32>;
@group(1) @binding(3) var t_mr: texture_2d<f32>;
@group(1) @binding(4) var s_mat: sampler;

@group(2) @binding(0) var<storage, read> models: array<Model>;

// Clustered Forward Rendering structures
struct PointLight {
    position: vec4<f32>,
    color: vec4<f32>,
    intensity: f32,
    radius: f32,
    _pad: vec2<f32>,
};

struct ClusterMetadata {
    offset: u32,
    count: u32,
};

struct ClusterInfo {
    clusters_x: u32,
    clusters_y: u32,
    depth_slices: u32,
    screen_width: u32,
    screen_height: u32,
    near: f32,
    far: f32,
    depth_scale: f32,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) uv:       vec2<f32>,
    @location(3) tangent:  vec3<f32>,
    @builtin(instance_index) instance_idx: u32,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos:    vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) world_tangent: vec3<f32>,
    @location(3) uv:           vec2<f32>,
    @location(4) instance_color: vec4<f32>,
    @location(5) shadow_pos:     vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let model = models[in.instance_idx].model;
    let world_pos = model * vec4<f32>(in.position, 1.0);
    out.clip_pos = camera.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    
    let normal_matrix = mat3x3<f32>(
        model[0].xyz,
        model[1].xyz,
        model[2].xyz
    );
    out.world_normal = normalize(normal_matrix * in.normal);
    out.world_tangent = normalize(normal_matrix * in.tangent);
    
    out.uv = in.uv;
    out.instance_color = models[in.instance_idx].color;
    out.shadow_pos = light_u.shadow_view_proj * world_pos;
    return out;
}

const PI: f32 = 3.14159265359;

fn fetch_shadow(shadow_coords: vec4<f32>) -> f32 {
    // Perspective divide
    let sc = shadow_coords.xyz / shadow_coords.w;
    // NDC (-1 to 1) to texture (0 to 1), flip Y
    let proj_coords = vec3<f32>(
        sc.x * 0.5 + 0.5,
        sc.y * -0.5 + 0.5,
        sc.z
    );
    
    if (proj_coords.z > 1.0) {
        return 1.0;
    }

    var visibility = 0.0;
    let size = 1.0 / 2048.0; // Shadow map resolution
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * size;
            visibility += textureSampleCompare(
                t_shadow, s_shadow, 
                proj_coords.xy + offset, 
                proj_coords.z - 0.005 // bias
            );
        }
    }
    return visibility / 9.0;
}

fn distribution_ggx(n: vec3<f32>, h: vec3<f32>, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let n_dot_h = max(dot(n, h), 0.0);
    let n_dot_h2 = n_dot_h * n_dot_h;
    let nom = a2;
    let denom = (n_dot_h2 * (a2 - 1.0) + 1.0);
    return nom / (PI * denom * denom);
}

fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let r = (roughness + 1.0);
    let k = (r * r) / 8.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, roughness: f32) -> f32 {
    let n_dot_v = max(dot(n, v), 0.0);
    let n_dot_l = max(dot(n, l), 0.0);
    let ggx2 = geometry_schlick_ggx(n_dot_v, roughness);
    let ggx1 = geometry_schlick_ggx(n_dot_l, roughness);
    return ggx1 * ggx2;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

fn fresnel_schlick_roughness(cos_theta: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    return f0 + (max(vec3<f32>(1.0 - roughness), f0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Clustered Forward Rendering helper functions
fn linearize_depth(ndc_depth: f32, near: f32, far: f32) -> f32 {
    let z_ndc = ndc_depth * 2.0 - 1.0; // Convert from [0, 1] to [-1, 1]
    return (2.0 * near * far) / ((far + near) - z_ndc * (far - near));
}

fn get_depth_slice(linear_depth: f32, cluster_info: ClusterInfo) -> u32 {
    let z_norm = log(linear_depth / cluster_info.near) / cluster_info.depth_scale;
    return min(u32(max(z_norm, 0.0)), cluster_info.depth_slices - 1u);
}

fn get_cluster_index(screen_pos: vec2<f32>, cluster_info: ClusterInfo) -> u32 {
    let cluster_x = u32(screen_pos.x / 16.0); // CLUSTER_SIZE_X = 16
    let cluster_y = u32(screen_pos.y / 16.0); // CLUSTER_SIZE_Y = 16
    
    if (cluster_x >= cluster_info.clusters_x || cluster_y >= cluster_info.clusters_y) {
        return 0xFFFFFFFFu; // Invalid index
    }
    
    return cluster_x + cluster_y * cluster_info.clusters_x;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var albedo = material.base_color.rgb * in.instance_color.rgb;
    if ((material.flags & 1u) != 0u) {
        albedo *= textureSample(t_albedo, s_mat, in.uv).rgb;
    }
    
    var n = normalize(in.world_normal);
    if ((material.flags & 2u) != 0u) {
        let t = normalize(in.world_tangent);
        let b = cross(n, t);
        let tbn = mat3x3<f32>(t, b, n);
        let normal_map = textureSample(t_normal, s_mat, in.uv).xyz * 2.0 - 1.0;
        n = normalize(tbn * normal_map);
    }
    
    var metallic = material.metallic;
    var roughness = material.roughness;
    if ((material.flags & 4u) != 0u) {
        let mr = textureSample(t_mr, s_mat, in.uv);
        metallic *= mr.b;
        roughness *= mr.g;
    }
    roughness = clamp(roughness, 0.05, 1.0);

    let v = normalize(camera.camera_pos.xyz - in.world_pos);
    let n_dot_v = max(dot(n, v), 0.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    var lo = vec3<f32>(0.0);

    // --- Directional Light ---
    {
        let l = normalize(-light_u.dir_direction.xyz);
        let h = normalize(v + l);
        let n_dot_l = max(dot(n, l), 0.0);
        
        let ndf = distribution_ggx(n, h, roughness);
        let g = geometry_smith(n, v, l, roughness);
        let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
        
        let numerator = ndf * g * f;
        let denominator = 4.0 * n_dot_v * n_dot_l + 0.0001;
        let specular = numerator / denominator;
        
        let radiance = light_u.dir_color.rgb * light_u.dir_color.w;
        let shadow = fetch_shadow(in.shadow_pos);
        
        let ks = f;
        let kd = (vec3<f32>(1.0) - ks) * (1.0 - metallic);
        lo += (kd * albedo / PI + specular) * radiance * n_dot_l * shadow;
    }

    // --- Point Lights (Clustered Forward Rendering) ---
    // Compute cluster coordinates
    let cluster_x = u32(in.clip_pos.x / 16.0);
    let cluster_y = u32(in.clip_pos.y / 16.0);
    
    // Compute linear depth for this fragment
    let cam_to_frag = length(camera.camera_pos.xyz - in.world_pos);
    let cluster_z = get_depth_slice(cam_to_frag, cluster_info);
    
    // Get cluster index in the grid
    if (cluster_x < cluster_info.clusters_x && cluster_y < cluster_info.clusters_y && cluster_z < cluster_info.depth_slices) {
        let cluster_idx = cluster_x + cluster_y * cluster_info.clusters_x + cluster_z * cluster_info.clusters_x * cluster_info.clusters_y;
        
        // Get light list for this cluster
        let meta = cluster_metadata[cluster_idx];
        
        // Iterate over lights in this cluster
        for (var i = 0u; i < meta.count; i = i + 1u) {
            let light_idx = cluster_light_indices[meta.offset + i];
            let light = point_lights[light_idx];
            
            let p_pos = light.position.xyz;
            let p_color = light.color.rgb;
            let p_intensity = light.intensity;
            let p_radius = light.radius;

            let l = normalize(p_pos - in.world_pos);
            let h = normalize(v + l);
            let dist = length(p_pos - in.world_pos);
            
            let attenuation = 1.0 / (dist * dist + 1.0) * (1.0 - smoothstep(0.0, p_radius, dist));
            let radiance = p_color * p_intensity;

            let n_dot_l = max(dot(n, l), 0.0);
            let ndf = distribution_ggx(n, h, roughness);
            let g = geometry_smith(n, v, l, roughness);
            let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
            
            let numerator = ndf * g * f;
            let denominator = 4.0 * n_dot_v * n_dot_l + 0.0001;
            let specular = numerator / denominator;
            
            let ks = f;
            let kd = (vec3<f32>(1.0) - ks) * (1.0 - metallic);
            lo += (kd * albedo / PI + specular) * radiance * n_dot_l * attenuation;
        }
    }

    // --- Ambient IBL ---
    let kS_ibl = fresnel_schlick_roughness(n_dot_v, f0, roughness);
    let kD_ibl = (vec3<f32>(1.0) - kS_ibl) * (1.0 - metallic);
    
    let irradiance = textureSample(t_irradiance, s_ibl, n).rgb;
    let diffuse_ibl = irradiance * albedo;
    
    let r = reflect(-v, n);
    let prefiltered_color = textureSampleLevel(t_prefiltered, s_ibl, r, roughness * 4.0).rgb;
    let env_brdf = textureSample(t_brdf_lut, s_ibl, vec2<f32>(n_dot_v, roughness)).rg;
    let specular_ibl = prefiltered_color * (kS_ibl * env_brdf.x + env_brdf.y);
    
    // SSAO пока отключено
    let ao = 1.0;

    // Ambient factor from light uniform (should be 1.0)
    var ambient_factor = light_u.ambient.w;
    if (ambient_factor < 0.01) {
        ambient_factor = 1.0; // fallback if zero
    }

    // Combine IBL with uniform ambient
    let ambient = (kD_ibl * diffuse_ibl + specular_ibl) * ambient_factor * ao + light_u.ambient.xyz * albedo * ao;
    let color = ambient + lo + material.emissive.rgb;
    
    // Fallback: if ambient is too dark, add a small constant to keep things visible
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    var final_color = color;
    if (luminance < 0.01) {
        final_color = albedo * 0.3; // dim but visible
    }
    
    return vec4<f32>(final_color, material.base_color.a * in.instance_color.a);
}
"#;

pub const SHADOW_SHADER_SRC: &str = r#"
struct ShadowMatrix {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> shadow: ShadowMatrix;

struct Model {
    model: mat4x4<f32>,
    color: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> models: array<Model>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @builtin(instance_index) instance_idx: u32,
};

@vertex
fn vs_main(input: VertexInput) -> @builtin(position) vec4<f32> {
    let model = models[input.instance_idx].model;
    return shadow.view_proj * model * vec4<f32>(input.position, 1.0);
}
"#;

pub const NORMAL_SHADER_SRC: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct Model {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> models: array<Model>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @builtin(instance_index) instance_idx: u32,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
};

const FAR: f32 = 1000.0;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let model = models[in.instance_idx].model;
    let world_pos = model * vec4<f32>(in.position, 1.0);
    out.clip_pos = camera.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    let normal_matrix = mat3x3<f32>(
        model[0].xyz,
        model[1].xyz,
        model[2].xyz
    );
    out.world_normal = normalize(normal_matrix * in.normal);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let linear_depth = length(camera.camera_pos.xyz - in.world_pos) / FAR;
    return vec4<f32>(in.world_normal, linear_depth);
}
"#;

pub const SSAO_SHADER_SRC: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

@group(0) @binding(0) var t_normal_depth: texture_2d<f32>;
@group(0) @binding(1) var t_noise: texture_2d<f32>;
@group(0) @binding(2) var s_ssao: sampler;
@group(0) @binding(3) var<uniform> kernel: array<vec4<f32>, 64>;
@group(0) @binding(4) var<uniform> camera: Camera;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) f32 {
    let normal_depth = textureSample(t_normal_depth, s_ssao, in.uv);
    let normal = normalize(normal_depth.rgb);
    let depth = normal_depth.a;

    // SSAO parameters
    let radius = 0.5;
    let bias = 0.025;
    let noise_scale = vec2<f32>(textureDimensions(t_normal_depth).xy) / 4.0;
    let random_vec = textureSample(t_noise, s_ssao, in.uv * noise_scale).xyz;

    // Tangent space basis
    let tangent = normalize(random_vec - normal * dot(random_vec, normal));
    let bitangent = cross(normal, tangent);
    let tbn = mat3x3<f32>(tangent, bitangent, normal);

    var occlusion = 0.0;
    for (var i = 0u; i < 64u; i++) {
        // From tangent to world space
        let sample_pos = tbn * kernel[i].xyz;
        // World space sample pos
        // Simplified: we only have normal and depth, so we reconstruct view-space pos is hard.
        // For the MVP SSAO, let's just do a simplified screen-space compare.
        
        let sample_uv = in.uv + sample_pos.xy * radius;
        let sample_depth = textureSample(t_normal_depth, s_ssao, sample_uv).a;
        
        let range_check = smoothstep(0.0, 1.0, radius / abs(depth - sample_depth));
        occlusion += select(0.0, 1.0, sample_depth >= (depth + bias)) * range_check;
    }

    return 1.0 - (occlusion / 64.0);
}
"#;

pub const SSAO_BLUR_SHADER_SRC: &str = r#"
@group(0) @binding(0) var t_ssao: texture_2d<f32>;
@group(0) @binding(1) var s_ssao: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) f32 {
    let tex_size = vec2<f32>(textureDimensions(t_ssao));
    let texel_size = 1.0 / tex_size;
    var result = 0.0;
    
    for (var x = -2; x < 2; x++) {
        for (var y = -2; y < 2; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            result += textureSample(t_ssao, s_ssao, in.uv + offset).r;
        }
    }
    
    return result / 16.0;
}
"#;

// ---------------------------------------------------------------------------
// IBL compute shaders – one module per entry point to avoid the
// @group(0) @binding(0) type conflict that existed in the monolithic file.
// When all four entry points shared one module, naga's per-entry-point
// binding resolution caused the BRDF-LUT pipeline to treat its storage
// texture as a sampled texture, so textureStore() never wrote anything,
// leaving env_brdf = (0,0) and specular_ibl = 0 for every fragment.
// ---------------------------------------------------------------------------

pub const EQUIRECT_TO_CUBE_SRC: &str = r#"
const PI: f32 = 3.14159265359;

@group(0) @binding(0) var input_equirect:  texture_2d<f32>;
@group(0) @binding(1) var samp:            sampler;
@group(0) @binding(2) var output_cubemap:  texture_storage_2d_array<rgba16float, write>;

fn get_cube_dir(id: vec3<u32>, size: vec2<u32>) -> vec3<f32> {
    let uv        = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    let tex_coords = uv * 2.0 - 1.0;
    var dir: vec3<f32>;
    let face = id.z;
    if      (face == 0u) { dir = vec3<f32>( 1.0, -tex_coords.y, -tex_coords.x); }
    else if (face == 1u) { dir = vec3<f32>(-1.0, -tex_coords.y,  tex_coords.x); }
    else if (face == 2u) { dir = vec3<f32>( tex_coords.x,  1.0,  tex_coords.y); }
    else if (face == 3u) { dir = vec3<f32>( tex_coords.x, -1.0, -tex_coords.y); }
    else if (face == 4u) { dir = vec3<f32>( tex_coords.x, -tex_coords.y,  1.0); }
    else                 { dir = vec3<f32>(-tex_coords.x, -tex_coords.y, -1.0); }
    return normalize(dir);
}

fn sample_equirect(v: vec3<f32>) -> vec2<f32> {
    let phi   = atan2(v.z, v.x);
    let theta = asin(v.y);
    return vec2<f32>(phi / (2.0 * PI) + 0.5, theta / PI + 0.5);
}

@compute @workgroup_size(8, 8, 1)
fn equirect_to_cubemap(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_cubemap).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }
    let color = textureSampleLevel(input_equirect, samp, sample_equirect(get_cube_dir(id, size)), 0.0);
    textureStore(output_cubemap, id.xy, i32(id.z), color);
}
"#;

pub const IRRADIANCE_CONVOLUTION_SRC: &str = r#"
const PI: f32 = 3.14159265359;

@group(0) @binding(0) var environment_map:    texture_cube<f32>;
@group(0) @binding(1) var samp:               sampler;
@group(1) @binding(0) var output_irradiance:  texture_storage_2d_array<rgba16float, write>;

fn get_cube_dir(id: vec3<u32>, size: vec2<u32>) -> vec3<f32> {
    let uv        = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    let tex_coords = uv * 2.0 - 1.0;
    var dir: vec3<f32>;
    let face = id.z;
    if      (face == 0u) { dir = vec3<f32>( 1.0, -tex_coords.y, -tex_coords.x); }
    else if (face == 1u) { dir = vec3<f32>(-1.0, -tex_coords.y,  tex_coords.x); }
    else if (face == 2u) { dir = vec3<f32>( tex_coords.x,  1.0,  tex_coords.y); }
    else if (face == 3u) { dir = vec3<f32>( tex_coords.x, -1.0, -tex_coords.y); }
    else if (face == 4u) { dir = vec3<f32>( tex_coords.x, -tex_coords.y,  1.0); }
    else                 { dir = vec3<f32>(-tex_coords.x, -tex_coords.y, -1.0); }
    return normalize(dir);
}

@compute @workgroup_size(8, 8, 1)
fn irradiance_convolution(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_irradiance).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }

    let normal = get_cube_dir(id, size);
    var irradiance = vec3<f32>(0.0);

    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let right = normalize(cross(up, normal));
    up = cross(normal, right);

    let sample_delta = 0.025;
    var nr_samples = 0.0;
    for (var phi = 0.0; phi < 2.0 * PI; phi += sample_delta) {
        for (var theta = 0.0; theta < 0.5 * PI; theta += sample_delta) {
            let ts  = vec3<f32>(sin(theta) * cos(phi), sin(theta) * sin(phi), cos(theta));
            let ws  = ts.x * right + ts.y * up + ts.z * normal;
            irradiance += textureSampleLevel(environment_map, samp, ws, 0.0).rgb
                          * cos(theta) * sin(theta);
            nr_samples += 1.0;
        }
    }
    irradiance = PI * irradiance / nr_samples;
    textureStore(output_irradiance, id.xy, i32(id.z), vec4<f32>(irradiance, 1.0));
}
"#;

pub const BRDF_LUT_SRC: &str = r#"
const PI: f32 = 3.14159265359;

@group(0) @binding(0) var output_lut: texture_storage_2d<rgba16float, write>;

fn radical_inverse_vdc(bits_in: u32) -> f32 {
    var bits = (bits_in << 16u) | (bits_in >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}
fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse_vdc(i));
}
fn importance_sample_ggx(xi: vec2<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let a  = roughness * roughness;
    let phi       = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    let h = vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(n.y) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let right = normalize(cross(up, n));
    up = cross(n, right);
    return normalize(right * h.x + up * h.y + n * h.z);
}
fn geometry_schlick_ggx_ibl(n_dot_v: f32, roughness: f32) -> f32 {
    let k = (roughness * roughness) / 2.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}
fn geometry_smith_ibl(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx_ibl(n_dot_v, roughness)
         * geometry_schlick_ggx_ibl(n_dot_l, roughness);
}

@compute @workgroup_size(8, 8, 1)
fn brdf_lut(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_lut);
    if (id.x >= size.x || id.y >= size.y) { return; }

    let n_dot_v   = max(f32(id.x) / f32(size.x), 0.001);
    let roughness = max(f32(id.y) / f32(size.y), 0.001);

    var v: vec3<f32>;
    v.x = sqrt(1.0 - n_dot_v * n_dot_v);
    v.y = 0.0;
    v.z = n_dot_v;

    var a = 0.0;
    var b = 0.0;
    let n = vec3<f32>(0.0, 0.0, 1.0);
    let sample_count = 1024u;

    for (var i = 0u; i < sample_count; i++) {
        let xi    = hammersley(i, sample_count);
        let h     = importance_sample_ggx(xi, n, roughness);
        let l     = normalize(2.0 * dot(v, h) * h - v);
        let n_dot_l = max(l.z, 0.0);
        let n_dot_h = max(h.z, 0.0);
        let v_dot_h = max(dot(v, h), 0.0);
        if (n_dot_l > 0.0) {
            let g     = geometry_smith_ibl(n_dot_v, n_dot_l, roughness);
            // Guard against division by zero
            let g_vis = (g * v_dot_h) / max(n_dot_h * n_dot_v, 0.0001);
            let fc    = pow(1.0 - v_dot_h, 5.0);
            a += (1.0 - fc) * g_vis;
            b += fc * g_vis;
        }
    }
    textureStore(output_lut, id.xy,
        vec4<f32>(a / f32(sample_count), b / f32(sample_count), 0.0, 1.0));
}
"#;

pub const PREFILTER_CONVOLUTION_SRC: &str = r#"
const PI: f32 = 3.14159265359;

@group(0) @binding(0) var environment_map:     texture_cube<f32>;
@group(0) @binding(1) var samp:                sampler;
@group(1) @binding(0) var output_prefiltered:  texture_storage_2d_array<rgba16float, write>;

struct PrefilterUniforms { roughness: f32 };
@group(2) @binding(0) var<uniform> prefilter_u: PrefilterUniforms;

fn get_cube_dir(id: vec3<u32>, size: vec2<u32>) -> vec3<f32> {
    let uv        = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    let tex_coords = uv * 2.0 - 1.0;
    var dir: vec3<f32>;
    let face = id.z;
    if      (face == 0u) { dir = vec3<f32>( 1.0, -tex_coords.y, -tex_coords.x); }
    else if (face == 1u) { dir = vec3<f32>(-1.0, -tex_coords.y,  tex_coords.x); }
    else if (face == 2u) { dir = vec3<f32>( tex_coords.x,  1.0,  tex_coords.y); }
    else if (face == 3u) { dir = vec3<f32>( tex_coords.x, -1.0, -tex_coords.y); }
    else if (face == 4u) { dir = vec3<f32>( tex_coords.x, -tex_coords.y,  1.0); }
    else                 { dir = vec3<f32>(-tex_coords.x, -tex_coords.y, -1.0); }
    return normalize(dir);
}
fn radical_inverse_vdc(bits_in: u32) -> f32 {
    var bits = (bits_in << 16u) | (bits_in >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}
fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse_vdc(i));
}
fn importance_sample_ggx(xi: vec2<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let a  = roughness * roughness;
    let phi       = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    let h = vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(n.y) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let right = normalize(cross(up, n));
    up = cross(n, right);
    return normalize(right * h.x + up * h.y + n * h.z);
}

@compute @workgroup_size(8, 8, 1)
fn prefilter_convolution(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_prefiltered).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }

    let n = get_cube_dir(id, size);
    let v = n;

    let sample_count = 1024u;
    var total_weight    = 0.0;
    var prefiltered_color = vec3<f32>(0.0);

    for (var i = 0u; i < sample_count; i++) {
        let xi    = hammersley(i, sample_count);
        let h     = importance_sample_ggx(xi, n, prefilter_u.roughness);
        let l     = normalize(2.0 * dot(v, h) * h - v);
        let n_dot_l = max(dot(n, l), 0.0);
        if (n_dot_l > 0.0) {
            prefiltered_color += textureSampleLevel(environment_map, samp, l, 0.0).rgb * n_dot_l;
            total_weight      += n_dot_l;
        }
    }
    prefiltered_color /= max(total_weight, 0.0001);
    textureStore(output_prefiltered, id.xy, i32(id.z), vec4<f32>(prefiltered_color, 1.0));
}
"#;

/// Creates a `wgpu::ShaderModule` from WGSL source code.
pub fn create_shader_module(
    device: &wgpu::Device,
    label: &str,
    source: &str,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}