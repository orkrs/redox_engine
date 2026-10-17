// Motion vectors of moving objects.
//
// The fullscreen velocity pass only knows how the CAMERA moved. This pass redraws the objects whose
// model matrix changed since the previous frame and overwrites their pixels with the full motion
// (camera + object), computed from the previous model matrix and the previous unjittered
// view-projection. Pixels are written only where the object is the visible surface (depth test
// against the forward pass depth), so hidden parts never leak their motion.

struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct Model {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

struct PrevModel {
    model: mat4x4<f32>,
};

struct VelocityUniform {
    inv_curr_vp : mat4x4<f32>,
    prev_vp     : mat4x4<f32>,
    screen_size : vec2<f32>,
    jitter_uv   : vec2<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> models: array<Model>;
@group(0) @binding(2) var<storage, read> prev_models: array<PrevModel>;
@group(0) @binding(3) var<uniform> vel_u: VelocityUniform;

struct VsOut {
    @builtin(position) clip_pos : vec4<f32>,
    @location(0)       prev_clip : vec4<f32>,
};

// Pulls the surface slightly towards the camera so that it passes the LessEqual depth test against
// the depth the forward pass wrote for the very same triangle.
const DEPTH_PULL: f32 = 2.0e-5;

@vertex
fn vs_main(in: CompressedVertIn, @builtin(instance_index) instance_idx: u32) -> VsOut {
    var out: VsOut;
    let mdl = models[instance_idx];
    let local_pos = decode_local_position(in.pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    let world_pos = mdl.model * vec4<f32>(local_pos, 1.0);
    let prev_world = prev_models[instance_idx].model * vec4<f32>(local_pos, 1.0);
    var clip = camera.view_proj * world_pos;
    clip.z = clip.z - DEPTH_PULL * clip.w;
    out.clip_pos = clip;
    out.prev_clip = vel_u.prev_vp * prev_world;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.clip_pos.xy / vel_u.screen_size;
    let prev_ndc = in.prev_clip.xy / in.prev_clip.w;
    let prev_uv = vec2<f32>(prev_ndc.x * 0.5 + 0.5, 0.5 - prev_ndc.y * 0.5);
    return vec4<f32>(uv - prev_uv, in.clip_pos.z, 1.0);
}
