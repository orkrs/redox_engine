struct Uniforms {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    light_dir: vec3<f32>,
    selected: f32,
    base_color: vec3<f32>,
    mode: u32,
    cam_pos: vec3<f32>,
    _pad: f32,
}
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_p: vec3<f32>,
    @location(1) world_n: vec3<f32>,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let wp = u.model * vec4<f32>(in.pos, 1.0);
    out.clip = u.mvp * vec4<f32>(in.pos, 1.0);
    out.world_p = wp.xyz;
    out.world_n = normalize((u.model * vec4<f32>(in.normal, 0.0)).xyz);
    return out;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

fn distribution_ggx(n: vec3<f32>, h: vec3<f32>, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let nh = max(dot(n, h), 0.0);
    let nh2 = nh * nh;
    let denom = nh2 * (a2 - 1.0) + 1.0;
    return a2 / (3.14159265 * denom * denom + 1e-5);
}

fn geometry_schlick_ggx(nv: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return nv / (nv * (1.0 - k) + k + 1e-5);
}

fn geometry_smith(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, roughness: f32) -> f32 {
    let nv = max(dot(n, v), 0.0);
    let nl = max(dot(n, l), 0.0);
    return geometry_schlick_ggx(nv, roughness) * geometry_schlick_ggx(nl, roughness);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_n);
    if (u.mode == 1u) {
        return vec4<f32>(u.base_color, 1.0);
    }
    if (u.mode == 2u) {
        return vec4<f32>(n * 0.5 + vec3<f32>(0.5), 1.0);
    }
    if (u.mode == 3u) {
        let rim = 1.0 - abs(dot(n, normalize(u.cam_pos - in.world_p)));
        let rgb = mix(u.base_color * 0.12, vec3<f32>(0.95, 0.95, 1.0), pow(rim, 1.5));
        return vec4<f32>(rgb, 1.0);
    }

    let albedo = u.base_color;
    let metallic = 0.05;
    let roughness = 0.45;
    let ao = 1.0;
    let v = normalize(u.cam_pos - in.world_p);
    let l = normalize(-u.light_dir);
    let h = normalize(v + l);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    let ndf = distribution_ggx(n, h, roughness);
    let g = geometry_smith(n, v, l, roughness);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
    let specular = (ndf * g * f) / (4.0 * max(dot(n, v), 0.0) * max(dot(n, l), 0.0) + 1e-4);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    let radiance = vec3<f32>(3.2, 3.0, 2.8);
    let ndotl = max(dot(n, l), 0.0);
    var color = (kd * albedo / 3.14159265 + specular) * radiance * ndotl;
    color = color + albedo * 0.08 * ao;

    if (u.selected > 0.5) {
        color = mix(color, vec3<f32>(1.0, 0.55, 0.12), 0.28);
        color = color + vec3<f32>(0.06, 0.03, 0.0);
    }
    return vec4<f32>(color, 1.0);
}
