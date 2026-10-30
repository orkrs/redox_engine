//! Renderer entry points that give global illumination the static scene.

use super::Renderer;
use crate::sdf::SdfBrick;
use crate::sdf::global::{GLOBAL_SDF_DIMS, GlobalSdf, SdfBox};
use redox_math::{Aabb, Vec3};

impl Renderer {
    /// Rebuilds the GI volume from static boxes: exact SDF, with emission from `emissive`.
    ///
    /// The volume covers the union of the boxes. The build runs on a worker thread and GI keeps
    /// the previous volume until the new one is uploaded. Call at level load. Boxes are
    /// axis-aligned; rotated objects cannot be represented and must not be passed here.
    /// An empty slice leaves the current volume unchanged.
    pub fn set_gi_static_geometry(&mut self, boxes: &[SdfBox]) {
        let Some(bounds) = union_bounds(boxes.iter().map(|b| (b.min, b.max))) else {
            return;
        };
        let boxes = boxes.to_vec();
        self.gi
            .spawn_rebuild(move || GlobalSdf::from_boxes(&boxes, bounds, GLOBAL_SDF_DIMS));
        self.gi_probes.set_enabled(true);
        self.gi_probes.notify_lighting_changed();
    }

    /// Rebuilds the GI volume from baked SDF bricks, each with colour `[r, g, b, emission]`.
    ///
    /// Same threading and limits as [`Self::set_gi_static_geometry`]: bricks are placed without
    /// rotation, so rotated objects cannot be represented.
    pub fn set_gi_static_bricks(&mut self, bricks: &[(SdfBrick, [f32; 4])]) {
        let domains = bricks.iter().map(|(brick, _)| {
            let half = brick.half_size();
            (brick.origin_ws - half, brick.origin_ws + half)
        });
        let Some(bounds) = union_bounds(domains) else {
            return;
        };
        let bricks = bricks.to_vec();
        self.gi
            .spawn_rebuild(move || GlobalSdf::from_bricks(&bricks, bounds, GLOBAL_SDF_DIMS));
        self.gi_probes.set_enabled(true);
        self.gi_probes.notify_lighting_changed();
    }
}

/// Smallest box containing every `(min, max)` pair, or `None` when there are none.
fn union_bounds(parts: impl Iterator<Item = (Vec3, Vec3)>) -> Option<Aabb> {
    parts.fold(None, |acc, (lo, hi)| {
        Some(match acc {
            None => Aabb { min: lo, max: hi },
            Some(a) => Aabb {
                min: a.min.min(lo),
                max: a.max.max(hi),
            },
        })
    })
}
