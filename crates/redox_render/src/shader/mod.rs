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
//! With the feature on, [`create_shader_module`] also resolves the embedded `*_WGSL`
//! constants to their on-disk files, so restarting the app picks up WGSL edits without
//! `cargo build`. [`ShaderWatcher`] detects saves while the app runs and reports WGSL
//! errors immediately. Pipelines are not rebuilt live: restart to see the change.

pub mod slang;
pub mod watch;
pub use slang::{SlangCompileOptions, SlangCompiler, SlangOutput, SlangTarget};
pub use watch::{ShaderReloadReport, ShaderWatcher};

/// Source of `forward.wgsl`.
pub const FORWARD_WGSL: &str = include_str!("../../shaders/forward.wgsl");
/// Source of `pbr.wgsl`.
pub const PBR_WGSL: &str = include_str!("../../shaders/pbr.wgsl");
/// Source of `shadow.wgsl`.
pub const SHADOW_WGSL: &str = include_str!("../../shaders/shadow.wgsl");
/// Source of `normal.wgsl`.
pub const NORMAL_WGSL: &str = include_str!("../../shaders/normal.wgsl");
/// Source of `gtao.wgsl`.
pub const GTAO_WGSL: &str = include_str!("../../shaders/gtao.wgsl");
/// Source of `gtao_blur.wgsl`.
pub const GTAO_BLUR_WGSL: &str = include_str!("../../shaders/gtao_blur.wgsl");
/// Source of `gtao_upsample.wgsl`.
pub const GTAO_UPSAMPLE_WGSL: &str = include_str!("../../shaders/gtao_upsample.wgsl");
/// Source of `vsm_shadow_trace.wgsl`.
pub const VSM_SHADOW_TRACE_WGSL: &str = include_str!("../../shaders/vsm_shadow_trace.wgsl");
/// Source of `vsm_shadow_temporal.wgsl`.
pub const VSM_SHADOW_TEMPORAL_WGSL: &str = include_str!("../../shaders/vsm_shadow_temporal.wgsl");
/// Source of `velocity.wgsl`.
pub const VELOCITY_WGSL: &str = include_str!("../../shaders/velocity.wgsl");
/// Source of `object_velocity.wgsl`.
pub const OBJECT_VELOCITY_WGSL: &str = include_str!("../../shaders/object_velocity.wgsl");
/// Source of `taa.wgsl`.
pub const TAA_WGSL: &str = include_str!("../../shaders/taa.wgsl");
/// Source of `msaa_resolve.wgsl`.
pub const MSAA_RESOLVE_WGSL: &str = include_str!("../../shaders/msaa_resolve.wgsl");
/// Source of `tone_mapping.wgsl`.
pub const TONE_MAPPING_WGSL: &str = include_str!("../../shaders/tone_mapping.wgsl");
/// Source of `equirect_to_cube.wgsl`.
pub const EQUIRECT_TO_CUBE_WGSL: &str = include_str!("../../shaders/equirect_to_cube.wgsl");
/// Source of `irradiance_convolution.wgsl`.
pub const IRRADIANCE_CONVOLUTION_WGSL: &str =
    include_str!("../../shaders/irradiance_convolution.wgsl");
/// Source of `prefilter_convolution.wgsl`.
pub const PREFILTER_CONVOLUTION_WGSL: &str =
    include_str!("../../shaders/prefilter_convolution.wgsl");
/// Source of `brdf_lut.wgsl`.
pub const BRDF_LUT_WGSL: &str = include_str!("../../shaders/brdf_lut.wgsl");
/// Source of `debug_lines.wgsl`.
pub const DEBUG_LINES_WGSL: &str = include_str!("../../shaders/debug_lines.wgsl");
/// Source of `vg.wgsl`.
pub const VG_WGSL: &str = include_str!("../../shaders/vg.wgsl");
/// Source of `vg_vis.wgsl`.
pub const VG_VIS_WGSL: &str = include_str!("../../shaders/vg_vis.wgsl");
/// Source of `vg_resolve.wgsl`.
pub const VG_RESOLVE_WGSL: &str = include_str!("../../shaders/vg_resolve.wgsl");
/// Source of `vg_shadow.wgsl`.
pub const VG_SHADOW_WGSL: &str = include_str!("../../shaders/vg_shadow.wgsl");
/// Source of `vg_cull.wgsl`.
pub const VG_CULL_WGSL: &str = include_str!("../../shaders/vg_cull.wgsl");
/// Source of `hiz_copy.wgsl`.
pub const HIZ_COPY_WGSL: &str = include_str!("../../shaders/hiz_copy.wgsl");
/// Source of `hiz_downsample.wgsl`.
pub const HIZ_DOWNSAMPLE_WGSL: &str = include_str!("../../shaders/hiz_downsample.wgsl");
/// Source of `vertex_compressed.wgsl`.
pub const VERTEX_COMPRESSED_WGSL: &str = include_str!("../../shaders/vertex_compressed.wgsl");
/// Source of `skybox.wgsl`.
pub const SKYBOX_WGSL: &str = include_str!("../../shaders/skybox.wgsl");
/// Source of `vsm_depth.wgsl`.
pub const VSM_DEPTH_WGSL: &str = include_str!("../../shaders/vsm_depth.wgsl");
/// Source of `vsm_visibility.wgsl`.
pub const VSM_VISIBILITY_WGSL: &str = include_str!("../../shaders/vsm_visibility.wgsl");
/// Source of `vsm_request.wgsl`.
pub const VSM_REQUEST_WGSL: &str = include_str!("../../shaders/vsm_request.wgsl");
/// Source of `scatter_light.wgsl`.
pub const SCATTER_LIGHT_WGSL: &str = include_str!("../../shaders/scatter_light.wgsl");
/// Source of `bloom.wgsl`.
pub const BLOOM_WGSL: &str = include_str!("../../shaders/bloom.wgsl");
/// Source of `cluster_cull_lights.wgsl`.
pub const CLUSTER_CULL_LIGHTS_WGSL: &str = include_str!("../../shaders/cluster_cull_lights.wgsl");
/// Source of `particle_sim.wgsl`.
pub const PARTICLE_SIM_WGSL: &str = include_str!("../../shaders/particle_sim.wgsl");
/// Source of `particle_billboard.wgsl`.
pub const PARTICLE_BILLBOARD_WGSL: &str = include_str!("../../shaders/particle_billboard.wgsl");
/// Source of `ddgi_trace.wgsl`.
pub const DDGI_TRACE_WGSL: &str = include_str!("../../shaders/ddgi_trace.wgsl");
/// Source of `smaa/edge.wgsl`.
pub const SMAA_EDGE_WGSL: &str = include_str!("../../shaders/smaa/edge.wgsl");
/// Source of `smaa/blend_weights.wgsl`.
pub const SMAA_BLEND_WGSL: &str = include_str!("../../shaders/smaa/blend_weights.wgsl");
/// Source of `smaa/neighborhood.wgsl`.
pub const SMAA_NEIGHBORHOOD_WGSL: &str = include_str!("../../shaders/smaa/neighborhood.wgsl");

/// Every embedded shader: (file stem, path relative to `shaders/`, embedded source).
///
/// Single source of truth for [`fallback`], [`load`], [`embedded_stem`] and [`ShaderWatcher`].
const EMBEDDED: &[(&str, &str, &str)] = &[
    ("forward", "forward.wgsl", FORWARD_WGSL),
    ("pbr", "pbr.wgsl", PBR_WGSL),
    ("shadow", "shadow.wgsl", SHADOW_WGSL),
    ("normal", "normal.wgsl", NORMAL_WGSL),
    ("gtao", "gtao.wgsl", GTAO_WGSL),
    ("gtao_blur", "gtao_blur.wgsl", GTAO_BLUR_WGSL),
    ("gtao_upsample", "gtao_upsample.wgsl", GTAO_UPSAMPLE_WGSL),
    (
        "vsm_shadow_trace",
        "vsm_shadow_trace.wgsl",
        VSM_SHADOW_TRACE_WGSL,
    ),
    (
        "vsm_shadow_temporal",
        "vsm_shadow_temporal.wgsl",
        VSM_SHADOW_TEMPORAL_WGSL,
    ),
    ("velocity", "velocity.wgsl", VELOCITY_WGSL),
    (
        "object_velocity",
        "object_velocity.wgsl",
        OBJECT_VELOCITY_WGSL,
    ),
    ("taa", "taa.wgsl", TAA_WGSL),
    ("msaa_resolve", "msaa_resolve.wgsl", MSAA_RESOLVE_WGSL),
    ("tone_mapping", "tone_mapping.wgsl", TONE_MAPPING_WGSL),
    (
        "equirect_to_cube",
        "equirect_to_cube.wgsl",
        EQUIRECT_TO_CUBE_WGSL,
    ),
    (
        "irradiance_convolution",
        "irradiance_convolution.wgsl",
        IRRADIANCE_CONVOLUTION_WGSL,
    ),
    (
        "prefilter_convolution",
        "prefilter_convolution.wgsl",
        PREFILTER_CONVOLUTION_WGSL,
    ),
    ("brdf_lut", "brdf_lut.wgsl", BRDF_LUT_WGSL),
    ("debug_lines", "debug_lines.wgsl", DEBUG_LINES_WGSL),
    ("vg", "vg.wgsl", VG_WGSL),
    ("vg_vis", "vg_vis.wgsl", VG_VIS_WGSL),
    ("vg_resolve", "vg_resolve.wgsl", VG_RESOLVE_WGSL),
    ("vg_shadow", "vg_shadow.wgsl", VG_SHADOW_WGSL),
    ("vg_cull", "vg_cull.wgsl", VG_CULL_WGSL),
    ("hiz_copy", "hiz_copy.wgsl", HIZ_COPY_WGSL),
    ("hiz_downsample", "hiz_downsample.wgsl", HIZ_DOWNSAMPLE_WGSL),
    (
        "vertex_compressed",
        "vertex_compressed.wgsl",
        VERTEX_COMPRESSED_WGSL,
    ),
    ("skybox", "skybox.wgsl", SKYBOX_WGSL),
    ("vsm_depth", "vsm_depth.wgsl", VSM_DEPTH_WGSL),
    ("vsm_visibility", "vsm_visibility.wgsl", VSM_VISIBILITY_WGSL),
    ("vsm_request", "vsm_request.wgsl", VSM_REQUEST_WGSL),
    ("scatter_light", "scatter_light.wgsl", SCATTER_LIGHT_WGSL),
    ("bloom", "bloom.wgsl", BLOOM_WGSL),
    (
        "cluster_cull_lights",
        "cluster_cull_lights.wgsl",
        CLUSTER_CULL_LIGHTS_WGSL,
    ),
    ("particle_sim", "particle_sim.wgsl", PARTICLE_SIM_WGSL),
    (
        "particle_billboard",
        "particle_billboard.wgsl",
        PARTICLE_BILLBOARD_WGSL,
    ),
    ("ddgi_trace", "ddgi_trace.wgsl", DDGI_TRACE_WGSL),
    ("smaa_edge", "smaa/edge.wgsl", SMAA_EDGE_WGSL),
    ("smaa_blend", "smaa/blend_weights.wgsl", SMAA_BLEND_WGSL),
    (
        "smaa_neighborhood",
        "smaa/neighborhood.wgsl",
        SMAA_NEIGHBORHOOD_WGSL,
    ),
];

/// Shaders that are compiled with the shared compressed-vertex decode prelude.
const WITH_VERTEX_DECODE: &[&str] = &[
    "pbr",
    "shadow",
    "normal",
    "object_velocity",
    "vg",
    "vg_vis",
    "vg_shadow",
    "vsm_depth",
];

/// Compile-time copy of a shader, keyed by file stem (e.g. `"pbr"` -> [`PBR_WGSL`]).
/// Unknown names yield an empty source; wgpu rejects it with a clear label at module creation.
#[cfg_attr(not(feature = "shader-hot-reload"), allow(unused))]
pub fn fallback(name: &str) -> &'static str {
    EMBEDDED
        .iter()
        .find(|(stem, _, _)| *stem == name)
        .map_or("", |(_, _, src)| *src)
}

/// File stem of the embedded shader whose source is exactly `src`, so callers that only
/// hold the `*_WGSL` constant can find the file behind it. Compared by content: `const`
/// strings are not guaranteed to share one address across use sites.
pub fn embedded_stem(src: &str) -> Option<&'static str> {
    EMBEDDED
        .iter()
        .find(|(_, _, embedded)| embedded.len() == src.len() && *embedded == src)
        .map(|(stem, _, _)| *stem)
}

/// Whether `stem` is built together with the compressed-vertex decode prelude.
pub fn needs_vertex_decode(stem: &str) -> bool {
    WITH_VERTEX_DECODE.contains(&stem)
}

/// Directory holding the `.wgsl` sources, relative to the crate manifest.
pub(crate) fn shaders_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Load a shader by file stem (e.g. `"pbr"`).
///
/// With `shader-hot-reload`: read `shaders/<path>.wgsl` from disk so an edited shader is
/// picked up without recompiling; fall back to the embedded copy if the file is unreadable.
///
/// Without the feature: return the embedded copy (zero I/O).
#[cfg(feature = "shader-hot-reload")]
pub fn load(name: &str) -> String {
    EMBEDDED
        .iter()
        .find(|(stem, _, _)| *stem == name)
        .and_then(|(_, rel, _)| std::fs::read_to_string(shaders_dir().join(rel)).ok())
        .unwrap_or_else(|| fallback(name).to_string())
}

/// Load a shader by file stem (see hot-reload variant).
#[cfg(not(feature = "shader-hot-reload"))]
pub fn load(name: &str) -> &'static str {
    fallback(name)
}

/// Source to compile for `src`: with `shader-hot-reload`, the on-disk version of an embedded
/// shader (so restarting the app picks up WGSL edits without `cargo build`); otherwise `src`.
#[cfg(feature = "shader-hot-reload")]
fn resolve_source(src: &str) -> std::borrow::Cow<'_, str> {
    match embedded_stem(src) {
        Some(stem) => std::borrow::Cow::Owned(load(stem)),
        None => std::borrow::Cow::Borrowed(src),
    }
}

#[cfg(not(feature = "shader-hot-reload"))]
fn resolve_source(src: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Borrowed(src)
}

/// Creates a `wgpu::ShaderModule` from WGSL source code.
pub fn create_shader_module(
    device: &wgpu::Device,
    label: &str,
    source: &str,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(resolve_source(source)),
    })
}

/// Prepends shared compressed-vertex decode helpers to a pass shader.
///
/// WGSL requires `enable ...;` directives before every other item, so they are hoisted
/// above the prelude.
pub fn shader_with_vertex_decode(source: &str) -> String {
    let (enables, body): (Vec<&str>, Vec<&str>) = source
        .lines()
        .partition(|l| l.trim_start().starts_with("enable "));
    format!(
        "{}\n{}\n{}",
        enables.join("\n"),
        resolve_source(VERTEX_COMPRESSED_WGSL),
        body.join("\n")
    )
}

/// Creates a shader module with compressed-vertex decode helpers included.
pub fn create_shader_with_vertex_decode(
    device: &wgpu::Device,
    label: &str,
    source: &str,
) -> wgpu::ShaderModule {
    create_shader_module(
        device,
        label,
        &shader_with_vertex_decode(&resolve_source(source)),
    )
}

#[cfg(test)]
mod tests {
    fn validate_wgsl(label: &str, source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{label}: parse failed: {e}"));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: validation failed: {e:?}"));
    }

    #[test]
    fn embedded_table_matches_files_and_constants() {
        for (stem, rel, src) in super::EMBEDDED {
            assert!(!src.is_empty(), "{stem}: embedded source is empty");
            assert!(
                super::shaders_dir().join(rel).is_file(),
                "{stem}: {rel} missing on disk"
            );
            assert_eq!(
                super::embedded_stem(src),
                Some(*stem),
                "{stem}: content lookup"
            );
        }
    }

    #[test]
    fn embedded_stem_ignores_foreign_sources() {
        assert_eq!(super::embedded_stem("fn main() {}"), None);
        assert_eq!(super::embedded_stem(super::PBR_WGSL), Some("pbr"));
    }

    #[test]
    fn vertex_decode_list_names_real_shaders() {
        for stem in super::WITH_VERTEX_DECODE {
            assert!(
                !super::fallback(stem).is_empty(),
                "{stem} is not an embedded shader"
            );
        }
    }

    #[test]
    fn all_embedded_shaders_validate() {
        let with_decode = [
            ("pbr", super::PBR_WGSL),
            ("shadow", super::SHADOW_WGSL),
            ("normal", super::NORMAL_WGSL),
            ("object_velocity", super::OBJECT_VELOCITY_WGSL),
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
            ("vsm_request", super::VSM_REQUEST_WGSL),
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
