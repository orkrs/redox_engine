//! Shader loading: compile-time [`include_str!`] with optional runtime hot-reload.
//!
//! Every built-in shader lives as a real `.wgsl` file under
//! `crates/redox_render/shaders/` and is embedded into the binary at compile
//! time via the `*_WGSL` constants below — a default build never touches the
//! filesystem for shaders.
//!
//! With the `shader-hot-reload` feature enabled, [`load`] reads the `.wgsl`
//! file from disk at runtime instead, falling back to the embedded copy when
//! the file is missing or unreadable.
//!
//! Note: this module currently only provides the load indirection. A watcher
//! that detects file changes and recreates `wgpu::ShaderModule`s (and the
//! pipelines depending on them) at runtime is planned as follow-up work.

pub mod slang;
pub use slang::{SlangCompiler, SlangCompileOptions, SlangTarget, SlangOutput};

pub const FORWARD_WGSL: &str = include_str!("../../shaders/forward.wgsl");
pub const PBR_WGSL: &str = include_str!("../../shaders/pbr.wgsl");
pub const SHADOW_WGSL: &str = include_str!("../../shaders/shadow.wgsl");
pub const NORMAL_WGSL: &str = include_str!("../../shaders/normal.wgsl");
pub const GTAO_WGSL: &str = include_str!("../../shaders/gtao.wgsl");
pub const GTAO_BLUR_WGSL: &str = include_str!("../../shaders/gtao_blur.wgsl");
pub const GTAO_UPSAMPLE_WGSL: &str = include_str!("../../shaders/gtao_upsample.wgsl");
pub const VSM_SHADOW_TRACE_WGSL: &str = include_str!("../../shaders/vsm_shadow_trace.wgsl");
pub const VSM_SHADOW_TEMPORAL_WGSL: &str = include_str!("../../shaders/vsm_shadow_temporal.wgsl");
pub const VELOCITY_WGSL: &str = include_str!("../../shaders/velocity.wgsl");
pub const TAA_WGSL: &str = include_str!("../../shaders/taa.wgsl");
pub const MSAA_RESOLVE_WGSL: &str = include_str!("../../shaders/msaa_resolve.wgsl");
pub const TONE_MAPPING_WGSL: &str = include_str!("../../shaders/tone_mapping.wgsl");
pub const EQUIRECT_TO_CUBE_WGSL: &str = include_str!("../../shaders/equirect_to_cube.wgsl");
pub const IRRADIANCE_CONVOLUTION_WGSL: &str =
    include_str!("../../shaders/irradiance_convolution.wgsl");
pub const PREFILTER_CONVOLUTION_WGSL: &str =
    include_str!("../../shaders/prefilter_convolution.wgsl");
pub const BRDF_LUT_WGSL: &str = include_str!("../../shaders/brdf_lut.wgsl");
pub const DEBUG_LINES_WGSL: &str = include_str!("../../shaders/debug_lines.wgsl");
pub const VG_WGSL: &str = include_str!("../../shaders/vg.wgsl");
pub const VG_VIS_WGSL: &str = include_str!("../../shaders/vg_vis.wgsl");
pub const VG_RESOLVE_WGSL: &str = include_str!("../../shaders/vg_resolve.wgsl");
pub const VG_SHADOW_WGSL: &str = include_str!("../../shaders/vg_shadow.wgsl");
pub const VG_CULL_WGSL: &str = include_str!("../../shaders/vg_cull.wgsl");
pub const HIZ_COPY_WGSL: &str = include_str!("../../shaders/hiz_copy.wgsl");
pub const HIZ_DOWNSAMPLE_WGSL: &str = include_str!("../../shaders/hiz_downsample.wgsl");
pub const VERTEX_COMPRESSED_WGSL: &str = include_str!("../../shaders/vertex_compressed.wgsl");
pub const SKYBOX_WGSL: &str = include_str!("../../shaders/skybox.wgsl");
pub const VSM_DEPTH_WGSL: &str = include_str!("../../shaders/vsm_depth.wgsl");
pub const VSM_VISIBILITY_WGSL: &str = include_str!("../../shaders/vsm_visibility.wgsl");
pub const SCATTER_LIGHT_WGSL: &str = include_str!("../../shaders/scatter_light.wgsl");
pub const BLOOM_WGSL: &str = include_str!("../../shaders/bloom.wgsl");
pub const CLUSTER_CULL_LIGHTS_WGSL: &str = include_str!("../../shaders/cluster_cull_lights.wgsl");
pub const PARTICLE_SIM_WGSL: &str = include_str!("../../shaders/particle_sim.wgsl");
pub const PARTICLE_BILLBOARD_WGSL: &str = include_str!("../../shaders/particle_billboard.wgsl");
pub const DDGI_TRACE_WGSL: &str = include_str!("../../shaders/ddgi_trace.wgsl");
pub const SMAA_EDGE_WGSL: &str = include_str!("../../shaders/smaa/edge.wgsl");
pub const SMAA_BLEND_WGSL: &str = include_str!("../../shaders/smaa/blend_weights.wgsl");
pub const SMAA_NEIGHBORHOOD_WGSL: &str = include_str!("../../shaders/smaa/neighborhood.wgsl");

/// Compile-time fallback for [`load`], keyed by shader file stem
/// (e.g. `"pbr"` → [`PBR_WGSL`]). Unknown names yield an empty source; wgpu
/// will reject it with a clear label at module creation.
#[cfg_attr(not(feature = "shader-hot-reload"), allow(unused))]
pub fn fallback(name: &str) -> &'static str {
    match name {
        "forward" => FORWARD_WGSL,
        "pbr" => PBR_WGSL,
        "shadow" => SHADOW_WGSL,
        "normal" => NORMAL_WGSL,
        "gtao" => GTAO_WGSL,
        "gtao_blur" => GTAO_BLUR_WGSL,
        "gtao_upsample" => GTAO_UPSAMPLE_WGSL,
        "velocity" => VELOCITY_WGSL,
        "taa" => TAA_WGSL,
        "msaa_resolve" => MSAA_RESOLVE_WGSL,
        "tone_mapping" => TONE_MAPPING_WGSL,
        "equirect_to_cube" => EQUIRECT_TO_CUBE_WGSL,
        "irradiance_convolution" => IRRADIANCE_CONVOLUTION_WGSL,
        "prefilter_convolution" => PREFILTER_CONVOLUTION_WGSL,
        "brdf_lut" => BRDF_LUT_WGSL,
        "debug_lines" => DEBUG_LINES_WGSL,
        "vg" => VG_WGSL,
        "vg_vis" => VG_VIS_WGSL,
        "vg_resolve" => VG_RESOLVE_WGSL,
        "vg_shadow" => VG_SHADOW_WGSL,
        "vg_cull" => VG_CULL_WGSL,
        "hiz_copy" => HIZ_COPY_WGSL,
        "hiz_downsample" => HIZ_DOWNSAMPLE_WGSL,
        "vertex_compressed" => VERTEX_COMPRESSED_WGSL,
        "skybox" => SKYBOX_WGSL,
        "vsm_depth" => VSM_DEPTH_WGSL,
        "vsm_visibility" => VSM_VISIBILITY_WGSL,
        "scatter_light" => SCATTER_LIGHT_WGSL,
        "bloom" => BLOOM_WGSL,
        "cluster_cull_lights" => CLUSTER_CULL_LIGHTS_WGSL,
        "particle_sim" => PARTICLE_SIM_WGSL,
        "particle_billboard" => PARTICLE_BILLBOARD_WGSL,
        "ddgi_trace" => DDGI_TRACE_WGSL,
        "smaa_edge" => SMAA_EDGE_WGSL,
        "smaa_blend" => SMAA_BLEND_WGSL,
        "smaa_neighborhood" => SMAA_NEIGHBORHOOD_WGSL,
        _ => "",
    }
}

/// Directory holding the `.wgsl` sources, relative to the crate manifest.
#[cfg(feature = "shader-hot-reload")]
fn shaders_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Load a shader by file stem (e.g. `"pbr"`).
///
/// With `shader-hot-reload`: read from `shaders/<name>.wgsl` on disk so an
/// edited shader can be picked up without recompiling; fall back to the
/// embedded copy if the file cannot be read.
///
/// Without the feature: return the embedded copy (zero I/O).
#[cfg(feature = "shader-hot-reload")]
pub fn load(name: &str) -> String {
    let path = shaders_dir().join(format!("{name}.wgsl"));
    std::fs::read_to_string(&path).unwrap_or_else(|_| fallback(name).to_string())
}

/// Load a shader by file stem (see hot-reload variant).
#[cfg(not(feature = "shader-hot-reload"))]
pub fn load(name: &str) -> &'static str {
    fallback(name)
}

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

/// Prepends shared compressed-vertex decode helpers to a pass shader.
pub fn shader_with_vertex_decode(source: &str) -> String {
    format!("{VERTEX_COMPRESSED_WGSL}\n{source}")
}

/// Creates a shader module with compressed-vertex decode helpers included.
pub fn create_shader_with_vertex_decode(
    device: &wgpu::Device,
    label: &str,
    source: &str,
) -> wgpu::ShaderModule {
    create_shader_module(device, label, &shader_with_vertex_decode(source))
}

#[cfg(test)]
mod tests {
    fn validate_wgsl(label: &str, source: &str) {
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| {
            panic!("{label}: parse failed: {e}")
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator.validate(&module).unwrap_or_else(|e| {
            panic!("{label}: validation failed: {e:?}")
        });
    }

    #[test]
    fn all_embedded_shaders_validate() {
        let with_decode = [
            ("pbr", super::PBR_WGSL),
            ("shadow", super::SHADOW_WGSL),
            ("normal", super::NORMAL_WGSL),
            ("vg", super::VG_WGSL),
            ("vg_vis", super::VG_VIS_WGSL),
            ("vg_shadow", super::VG_SHADOW_WGSL),
            ("vsm_depth", super::VSM_DEPTH_WGSL),
        ];
        for (name, src) in with_decode {
            validate_wgsl(name, &super::shader_with_vertex_decode(src));
        }

        let standalone = [
            ("gtao", super::GTAO_WGSL),
            ("gtao_blur", super::GTAO_BLUR_WGSL),
            ("gtao_upsample", super::GTAO_UPSAMPLE_WGSL),
            ("velocity", super::VELOCITY_WGSL),
            ("taa", super::TAA_WGSL),
            ("msaa_resolve", super::MSAA_RESOLVE_WGSL),
            ("tone_mapping", super::TONE_MAPPING_WGSL),
            ("debug_lines", super::DEBUG_LINES_WGSL),
            ("vg_resolve", super::VG_RESOLVE_WGSL),
            ("vg_cull", super::VG_CULL_WGSL),
            ("hiz_copy", super::HIZ_COPY_WGSL),
            ("hiz_downsample", super::HIZ_DOWNSAMPLE_WGSL),
            ("skybox", super::SKYBOX_WGSL),
            ("vsm_visibility", super::VSM_VISIBILITY_WGSL),
            ("scatter_light", super::SCATTER_LIGHT_WGSL),
            ("bloom", super::BLOOM_WGSL),
            ("cluster_cull_lights", super::CLUSTER_CULL_LIGHTS_WGSL),
            ("particle_sim", super::PARTICLE_SIM_WGSL),
            ("particle_billboard", super::PARTICLE_BILLBOARD_WGSL),
            ("ddgi_trace", super::DDGI_TRACE_WGSL),
            ("smaa_edge", super::SMAA_EDGE_WGSL),
            ("smaa_blend", super::SMAA_BLEND_WGSL),
            ("smaa_neighborhood", super::SMAA_NEIGHBORHOOD_WGSL),
        ];
        for (name, src) in standalone {
            validate_wgsl(name, src);
        }
    }
}
