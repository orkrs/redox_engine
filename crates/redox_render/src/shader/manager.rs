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
    point_lights_pos:   array<vec4<f32>, 3>,
    point_lights_color: array<vec4<f32>, 3>,
    num_point_lights:   u32,
    pad0:               u32,
    pad1:               u32,
    pad2:               u32,
};

struct ModelUniform {
    model: mat4x4<f32>,
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
    @builtin(instance_index) instance_index: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv:           vec2<f32>,
    @location(2) world_pos:    vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let model = models[in.instance_index].model;
    let world_pos = model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.world_normal = normalize((model * vec4<f32>(in.normal, 0.0)).xyz);
    out.uv = in.uv;
    out.world_pos = world_pos.xyz;
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

    let final_color = (light_u.ambient.xyz + total_diffuse) * tex_color.xyz;
    return vec4<f32>(final_color, tex_color.a);
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
