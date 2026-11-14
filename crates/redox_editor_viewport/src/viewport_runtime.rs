//! Applies [`ViewportRuntimeSettings`] to [`Renderer`].

use redox_render::Renderer;
use redox_render::gi::probes::GiDebug;
use redox_render::post::fog::FogSettings;
use redox_render::sky::SkyMode;

use redox_editor_core::viewport_prefs::{ViewportRuntimeSettings, ViewportToggleStatus};

/// Pushes editor viewport toggles into the engine; returns overlay hints for unavailable paths.
pub fn apply_viewport_runtime(
    renderer: &mut Renderer,
    cfg: &ViewportRuntimeSettings,
) -> ViewportToggleStatus {
    let mut status = ViewportToggleStatus::default();

    if cfg.procedural_sky {
        renderer.set_sky_mode(SkyMode::Procedural);
        if renderer.ibl_resource.is_dummy() {
            status.procedural_sky =
                "Sky: procedural on; IBL cube still dummy until prepare_procedural_sky runs";
        }
    } else {
        renderer.set_sky_mode(SkyMode::Hdri);
    }

    renderer.set_gi_enabled(cfg.gi_enabled);
    if cfg.gi_enabled && !renderer.gi_probes.enabled() {
        status.gi = "GI: requested on but no static SDF seeded in editor";
    }

    if cfg.fog_enabled {
        renderer.enable_taa(true);
        renderer.set_fog(Some(FogSettings::default()));
        renderer.set_fog_froxel(true);
        if !renderer.taa_enabled {
            status.fog = "Fog: froxel fog needs TAA branch (enable_taa failed)";
        }
    } else {
        renderer.set_fog(None);
        renderer.set_fog_froxel(false);
    }

    if cfg.sdf_debug {
        renderer.set_gi_debug(GiDebug::Visibility);
        status.sdf_debug = "SDF debug: GI visibility view (no dedicated SDF buffer)";
    } else {
        renderer.set_gi_debug(GiDebug::Off);
    }

    renderer.vg_use_vis_path = cfg.vg_enabled;
    status
}
