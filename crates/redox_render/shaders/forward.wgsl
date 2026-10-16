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
    // Inverse-transpose of model (upper-left 3x3) - matches pbr.wgsl Model.
    normal_matrix: mat3x3<f32>,
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
