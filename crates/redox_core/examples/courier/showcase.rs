//! Graphics of the courier demo that go beyond the basic forward frame: contact-hardening
//! shadows, screen-space reflections, height fog with sun scatter, global illumination from the
//! level's boxes and a light motion blur.

use redox_math::Vec3;
use redox_render::GiQuality;
use redox_render::Renderer;
use redox_render::post::fog::FogSettings;
use redox_render::post::post_fx::MotionBlurSettings;
use redox_render::post::ssr::SsrSettings;
use redox_render::sdf::global::SdfBox;
use redox_render::shadow::smrt::SmrtQuality;

use crate::game::Level;

/// Thickness of the ground slab the GI volume sees.
const GROUND_THICKNESS: f32 = 0.25;

/// Switches the new rendering features on. `gi` builds the GI volume from the arena and crates.
pub fn apply(r: &mut Renderer, level: &Level, gi: bool) {
    r.set_smrt_quality(SmrtQuality::Quality);
    r.set_ssr(Some(SsrSettings {
        max_roughness: 0.45,
        intensity: 0.7,
        ..Default::default()
    }));
    r.set_fog(Some(FogSettings {
        color: [0.62, 0.72, 0.85],
        density: 0.012,
        height_falloff: 0.2,
        sun_scatter: 0.6,
        ..Default::default()
    }));
    r.set_fog_froxel(true);
    r.set_motion_blur(Some(MotionBlurSettings {
        shutter: 0.35,
        max_pixels: 14.0,
        taps: 8,
        spread_beyond_edges: false,
    }));
    if gi {
        r.set_gi_quality(GiQuality::Medium);
        r.set_gi_static_geometry(&level_boxes(level));
        r.set_gi_enabled(true);
    }
}

/// The arena slab and every crate as axis-aligned boxes with their albedo.
fn level_boxes(level: &Level) -> Vec<SdfBox> {
    let half = level.half;
    let mut boxes = vec![SdfBox {
        min: Vec3::new(-half, -GROUND_THICKNESS, -half),
        max: Vec3::new(half, 0.0, half),
        albedo: Vec3::new(0.30, 0.42, 0.28),
        emissive: Vec3::ZERO,
    }];
    boxes.extend(level.crates.iter().map(|c| SdfBox {
        min: Vec3::new(c.x - c.half, 0.0, c.z - c.half),
        max: Vec3::new(c.x + c.half, c.half * 2.0, c.z + c.half),
        albedo: Vec3::new(0.55, 0.38, 0.22),
        emissive: Vec3::ZERO,
    }));
    boxes
}
