//! Sparse brick-atlas registry (CPU side).
//!
//! [`SdfBrickAtlas`] maps mesh ids to baked 64³ distance-field bricks. It is
//! the **CPU-side registry**. GPU upload lives in [`crate::sdf::gpu_atlas`]:
//! up to 8 bricks are packed into a 128³ atlas with a page table consumed by
//! `pbr.wgsl` / `sdf_shadow.wgsl`.
//!
//! GPU-upload notes for the future task:
//! * Pack each brick's `DistanceField64::data` into a single-channel format
//!   (`r16snorm` halves memory vs `r32float` with no practical precision
//!   loss for shadow tracing; keep `r32float` as the debug path).
//! * A brick occupies a contiguous 64³ sub-region ("page") of the atlas;
//!   [`BrickRef::index`] already provides a stable slot number to hang a
//!   page coordinate off.
//! * The WGSL companion (`docs/reference/shaders/sdf_shadow.wgsl`) and `pbr.wgsl` sample the
//!   GPU atlas via the multi-brick page table in [`crate::sdf::SdfSceneGpu`].

use super::bake::DistanceField64;
use redox_math::Vec3;
use std::collections::HashMap;

/// One placed distance-field brick.
#[derive(Clone, Debug)]
pub struct SdfBrick {
    /// Baked signed distance field (64³ texels).
    pub field: DistanceField64,
    /// World-space centre of the brick's domain cube.
    pub origin_ws: Vec3,
    /// Side length of the brick's domain cube in meters.
    pub world_size: Vec3,
}

impl SdfBrick {
    /// Half-extent of the domain cube.
    #[inline]
    pub fn half_size(&self) -> Vec3 {
        self.world_size * 0.5
    }

    /// True when `p` (world space) lies inside the brick's domain.
    #[inline]
    pub fn contains(&self, p: Vec3) -> bool {
        let d = (p - self.origin_ws).abs();
        d.x <= self.half_size().x && d.y <= self.half_size().y && d.z <= self.half_size().z
    }

    /// Trilinear signed-distance sample at world-space `p`. Only meaningful
    /// when [`Self::contains`] holds; outside positions clamp to the border
    /// texels.
    #[inline]
    pub fn sample_world(&self, p: Vec3) -> f32 {
        self.field.sample(p - self.origin_ws)
    }
}

/// Stable handle from a mesh id to its brick slot in [`SdfBrickAtlas`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrickRef {
    /// Index into [`SdfBrickAtlas::bricks`].
    pub index: u32,
}

/// Registry of baked SDF bricks keyed by mesh id.
///
/// Re-inserting an existing id replaces that brick **in place**, keeping
/// every other [`BrickRef`] valid — important once refs are handed out to
/// render objects.
#[derive(Clone, Debug, Default)]
pub struct SdfBrickAtlas {
    entries: HashMap<u64, BrickRef>,
    bricks: Vec<SdfBrick>,
}

impl SdfBrickAtlas {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts (or replaces) the brick for `id`.
    pub fn insert(&mut self, id: u64, brick: SdfBrick) {
        match self.entries.get(&id) {
            Some(&BrickRef { index }) => self.bricks[index as usize] = brick,
            None => {
                let index = self.bricks.len() as u32;
                self.bricks.push(brick);
                self.entries.insert(id, BrickRef { index });
            }
        }
    }

    /// Returns the brick registered for `id`, if any.
    pub fn get(&self, id: u64) -> Option<&SdfBrick> {
        self.entries
            .get(&id)
            .map(|r| &self.bricks[r.index as usize])
    }

    /// Number of bricks in the registry.
    #[inline]
    pub fn len(&self) -> usize {
        self.bricks.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bricks.is_empty()
    }

    /// Iterates all placed bricks (slot order).
    pub fn iter(&self) -> impl Iterator<Item = &SdfBrick> {
        self.bricks.iter()
    }

    /// Total CPU memory occupied by field data, in bytes (metadata excluded).
    pub fn total_bytes(&self) -> usize {
        self.bricks.iter().map(|b| b.field.data.len() * 4).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive::create_cube;

    #[test]
    fn atlas_insert_get_roundtrip() {
        // Bake a real field so byte accounting reflects production use.
        let cube = create_cube();
        let field = crate::sdf::bake_mesh(&cube, Vec3::splat(2.0));

        let mut atlas = SdfBrickAtlas::new();
        assert!(atlas.is_empty());
        assert_eq!(atlas.total_bytes(), 0);
        assert!(atlas.get(42).is_none());

        atlas.insert(
            42,
            SdfBrick {
                field: field.clone(),
                origin_ws: Vec3::new(10.0, 0.0, -5.0),
                world_size: Vec3::splat(2.0),
            },
        );
        assert_eq!(atlas.len(), 1);
        assert_eq!(atlas.total_bytes(), 64 * 64 * 64 * 4);

        let brick = atlas.get(42).expect("brick must round-trip");
        assert_eq!(brick.origin_ws, Vec3::new(10.0, 0.0, -5.0));
        assert_eq!(brick.world_size, Vec3::splat(2.0));
        assert_eq!(brick.field.data[0], field.data[0]);
        assert!(brick.contains(Vec3::new(10.0, 0.9, -5.0)));
        assert!(!brick.contains(Vec3::new(13.0, 0.0, -5.0)));

        // Re-inserting replaces in place instead of appending a slot.
        atlas.insert(
            42,
            SdfBrick {
                field,
                origin_ws: Vec3::ZERO,
                world_size: Vec3::splat(4.0),
            },
        );
        assert_eq!(atlas.len(), 1);
        assert_eq!(atlas.get(42).unwrap().origin_ws, Vec3::ZERO);

        // Distinct ids get distinct slots.
        atlas.insert(
            7,
            SdfBrick {
                field: DistanceField64 {
                    data: vec![1.0; 64 * 64 * 64],
                    world_size: Vec3::ONE,
                },
                origin_ws: Vec3::ZERO,
                world_size: Vec3::ONE,
            },
        );
        assert_eq!(atlas.len(), 2);
        assert_eq!(atlas.total_bytes(), 2 * 64 * 64 * 64 * 4);
    }
}
