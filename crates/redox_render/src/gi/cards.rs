//! Surface cards (GI-02 step 1): the layout shared by the CPU capture, the GPU atlas and the
//! trace lookup.
//!
//! A card is an orthographic snapshot of an object's world AABB along one of six axes. The face
//! `PosX` sits on the `max.x` plane and depth runs inward; `NegX` sits on `min.x`, and so on.
//! Card `(u, v)` are the normalised coordinates of the two axes spanning the face, and `depth` is
//! the normalised distance from the face (0 on the face, 1 on the far side).
//!
//! The WGSL mirror of `project_to_card`, `face_weights` and `unproject_from_card` lives in
//! `shaders/gi_common.wgsl`; keep them in sync.

use redox_math::{Vec2, Vec3};

/// Faces per object.
pub const CARD_FACES: usize = 6;
/// Smallest card edge in texels.
pub const CARD_MIN: u32 = 8;
/// Largest card edge in texels.
pub const CARD_MAX: u32 = 128;
/// Texel depth value that marks an empty texel.
pub const DEPTH_EMPTY: u8 = 255;

/// One of the six axis-aligned faces of an object's AABB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardFace {
    /// Face on the maximum X plane.
    PosX,
    /// Face on the minimum X plane.
    NegX,
    /// Face on the maximum Y plane.
    PosY,
    /// Face on the minimum Y plane.
    NegY,
    /// Face on the maximum Z plane.
    PosZ,
    /// Face on the minimum Z plane.
    NegZ,
}

impl CardFace {
    /// Every face in index order (`index()` 0..6).
    pub const ALL: [CardFace; CARD_FACES] = [
        CardFace::PosX,
        CardFace::NegX,
        CardFace::PosY,
        CardFace::NegY,
        CardFace::PosZ,
        CardFace::NegZ,
    ];

    /// Position of the face in [`Self::ALL`]; the order of `ObjectCards::faces`.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Outward unit normal of the face.
    pub fn normal(self) -> Vec3 {
        match self {
            CardFace::PosX => Vec3::X,
            CardFace::NegX => Vec3::NEG_X,
            CardFace::PosY => Vec3::Y,
            CardFace::NegY => Vec3::NEG_Y,
            CardFace::PosZ => Vec3::Z,
            CardFace::NegZ => Vec3::NEG_Z,
        }
    }

    /// `(u axis, v axis, depth axis)` as 0/1/2 indices. `u` and `v` span the card; `depth` is the
    /// axis the view runs along, from the face inward.
    pub fn axes(self) -> (usize, usize, usize) {
        match self {
            CardFace::PosX | CardFace::NegX => (1, 2, 0),
            CardFace::PosY | CardFace::NegY => (0, 2, 1),
            CardFace::PosZ | CardFace::NegZ => (0, 1, 2),
        }
    }

    /// True for the faces on the maximum side of their axis.
    fn is_positive(self) -> bool {
        matches!(self, CardFace::PosX | CardFace::PosY | CardFace::PosZ)
    }
}

/// Rectangle in the card atlas, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    /// Left texel.
    pub x: u32,
    /// Top texel.
    pub y: u32,
    /// Width in texels.
    pub w: u32,
    /// Height in texels.
    pub h: u32,
}

/// Texel size of a card: `ceil(extent / texel_world)` clamped to `[CARD_MIN, CARD_MAX]` and
/// rounded up to a multiple of four. A non-positive or non-finite texel size gives the maximum.
pub fn card_size(extent_u: f32, extent_v: f32, texel_world: f32) -> (u32, u32) {
    let side = |extent: f32| {
        let texels = extent / texel_world;
        if !(texel_world > 0.0) || !texels.is_finite() {
            return CARD_MAX;
        }
        let n = texels.ceil().clamp(0.0, CARD_MAX as f32) as u32;
        (n.div_ceil(4) * 4).clamp(CARD_MIN, CARD_MAX)
    };
    (side(extent_u), side(extent_v))
}

/// Shelf packer for the card atlas. Items fill a shelf left to right; a new shelf opens below
/// when no existing shelf is tall enough and has room.
#[derive(Clone, Debug)]
pub struct ShelfPacker {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    /// First free row below the last shelf.
    next_y: u32,
    /// Sum of the areas of placed rectangles, in texels.
    used_area: u64,
}

#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    used_x: u32,
}

impl ShelfPacker {
    /// Empty atlas of `w` by `h` texels.
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            width: w,
            height: h,
            shelves: Vec::new(),
            next_y: 0,
            used_area: 0,
        }
    }

    /// Places a `w` by `h` rectangle, or returns `None` when it does not fit.
    pub fn alloc(&mut self, w: u32, h: u32) -> Option<AtlasRect> {
        if w == 0 || h == 0 || w > self.width || h > self.height {
            return None;
        }
        let width = self.width;
        let found = self
            .shelves
            .iter_mut()
            .find(|s| s.height >= h && s.used_x + w <= width);
        let rect = if let Some(shelf) = found {
            let rect = AtlasRect {
                x: shelf.used_x,
                y: shelf.y,
                w,
                h,
            };
            shelf.used_x += w;
            rect
        } else {
            if self.next_y + h > self.height {
                return None;
            }
            let rect = AtlasRect {
                x: 0,
                y: self.next_y,
                w,
                h,
            };
            self.shelves.push(Shelf {
                y: self.next_y,
                height: h,
                used_x: w,
            });
            self.next_y += h;
            rect
        };
        self.used_area += u64::from(w) * u64::from(h);
        Some(rect)
    }

    /// Fraction of the atlas area covered by placed rectangles.
    pub fn used_fraction(&self) -> f32 {
        let total = u64::from(self.width) * u64::from(self.height);
        if total == 0 {
            return 0.0;
        }
        self.used_area as f32 / total as f32
    }
}

/// Projects a world point onto the card of `face` of an object with AABB `[aabb_min, aabb_max]`.
/// Returns `(uv, depth)`: `uv` is in `0..1` across the face and `depth` is 0 on the face and 1 on
/// the far side (values outside the box are not clamped).
pub fn project_to_card(face: CardFace, aabb_min: Vec3, aabb_max: Vec3, p: Vec3) -> (Vec2, f32) {
    let (u, v, d) = face.axes();
    let lo = aabb_min.to_array();
    let hi = aabb_max.to_array();
    let pa = p.to_array();
    let extent = |axis: usize| (hi[axis] - lo[axis]).max(1e-6);
    let uv = Vec2::new(
        (pa[u] - lo[u]) / extent(u),
        (pa[v] - lo[v]) / extent(v),
    );
    let depth = if face.is_positive() {
        (hi[d] - pa[d]) / extent(d)
    } else {
        (pa[d] - lo[d]) / extent(d)
    };
    (uv, depth)
}

/// Inverse of [`project_to_card`]: the world point at card coordinates `uv` and `depth`.
pub fn unproject_from_card(face: CardFace, aabb_min: Vec3, aabb_max: Vec3, uv: Vec2, depth: f32) -> Vec3 {
    let (u, v, d) = face.axes();
    let lo = aabb_min.to_array();
    let hi = aabb_max.to_array();
    let extent = |axis: usize| (hi[axis] - lo[axis]).max(1e-6);
    let mut p = [0.0f32; 3];
    p[u] = lo[u] + uv.x * extent(u);
    p[v] = lo[v] + uv.y * extent(v);
    p[d] = if face.is_positive() {
        hi[d] - depth * extent(d)
    } else {
        lo[d] + depth * extent(d)
    };
    Vec3::from_array(p)
}

/// Blend weights of the six faces for a surface normal: `max(n·face_normal, 0)²`, normalised to
/// sum one. A zero normal gives equal weights.
pub fn face_weights(n: Vec3) -> [f32; CARD_FACES] {
    let mut w = CardFace::ALL.map(|f| n.dot(f.normal()).max(0.0).powi(2));
    let sum: f32 = w.iter().sum();
    if sum <= 0.0 {
        return [1.0 / CARD_FACES as f32; CARD_FACES];
    }
    for weight in &mut w {
        *weight /= sum;
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random values in `[0, 1)` (LCG, no external crate).
    fn lcg(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*state >> 33) as f32) / ((1u64 << 31) as f32)
    }

    #[test]
    fn the_packer_never_overlaps_and_stays_inside() {
        let mut packer = ShelfPacker::new(2048, 2048);
        let mut state = 7u64;
        let mut placed = Vec::new();
        for _ in 0..500 {
            let w = 8 + 4 * ((lcg(&mut state) * 30.0) as u32);
            let h = 8 + 4 * ((lcg(&mut state) * 30.0) as u32);
            if let Some(r) = packer.alloc(w, h) {
                assert!(r.x + r.w <= 2048 && r.y + r.h <= 2048);
                placed.push(r);
            }
        }
        assert!(placed.len() > 400, "packer placed only {}", placed.len());
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                let apart = a.x + a.w <= b.x
                    || b.x + b.w <= a.x
                    || a.y + a.h <= b.y
                    || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn the_packer_reports_a_full_atlas_with_none() {
        let mut packer = ShelfPacker::new(64, 64);
        assert!(packer.alloc(64, 64).is_some());
        assert!(packer.alloc(8, 8).is_none());
        assert!((packer.used_fraction() - 1.0).abs() < 1e-6);
        assert!(packer.alloc(0, 8).is_none());
        assert!(packer.alloc(65, 8).is_none());
    }

    #[test]
    fn card_sizes_are_clamped_and_multiples_of_four() {
        assert_eq!(card_size(0.1, 0.1, 0.5), (CARD_MIN, CARD_MIN));
        assert_eq!(card_size(1000.0, 3.0, 0.1), (CARD_MAX, 32));
        let (w, h) = card_size(1.37, 2.2, 0.1);
        assert_eq!(w % 4, 0);
        assert_eq!(h % 4, 0);
        assert_eq!((w, h), (16, 24));
        assert_eq!(card_size(1.0, 1.0, 0.0), (CARD_MAX, CARD_MAX));
    }

    #[test]
    fn project_to_card_maps_the_face_corners_to_the_uv_corners() {
        let lo = Vec3::new(-1.0, 0.0, 2.0);
        let hi = Vec3::new(1.0, 3.0, 6.0);
        for face in CardFace::ALL {
            let (u, v, d) = face.axes();
            let fixed = if face.is_positive() { hi[d] } else { lo[d] };
            for (su, sv) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)] {
                let mut p = [0.0f32; 3];
                p[d] = fixed;
                p[u] = if su == 0 { lo[u] } else { hi[u] };
                p[v] = if sv == 0 { lo[v] } else { hi[v] };
                let (uv, depth) = project_to_card(face, lo, hi, Vec3::from_array(p));
                assert!((uv.x - su as f32).abs() < 1e-5, "{face:?} u");
                assert!((uv.y - sv as f32).abs() < 1e-5, "{face:?} v");
                assert!(depth.abs() < 1e-5, "{face:?} depth on the face");
            }
        }
    }

    #[test]
    fn the_depth_is_zero_on_the_face_and_one_on_the_far_side() {
        let lo = Vec3::ZERO;
        let hi = Vec3::new(2.0, 2.0, 2.0);
        let (_, d_pos) = project_to_card(CardFace::PosX, lo, hi, Vec3::new(2.0, 1.0, 1.0));
        let (_, d_far) = project_to_card(CardFace::PosX, lo, hi, Vec3::new(0.0, 1.0, 1.0));
        assert!(d_pos.abs() < 1e-6);
        assert!((d_far - 1.0).abs() < 1e-6);
        let (_, n_face) = project_to_card(CardFace::NegX, lo, hi, Vec3::new(0.0, 1.0, 1.0));
        let (_, n_far) = project_to_card(CardFace::NegX, lo, hi, Vec3::new(2.0, 1.0, 1.0));
        assert!(n_face.abs() < 1e-6);
        assert!((n_far - 1.0).abs() < 1e-6);
    }

    #[test]
    fn unproject_inverts_project() {
        let lo = Vec3::new(-0.5, 1.0, 3.0);
        let hi = Vec3::new(1.5, 4.0, 4.0);
        let p = Vec3::new(0.3, 2.2, 3.7);
        for face in CardFace::ALL {
            let (uv, depth) = project_to_card(face, lo, hi, p);
            let back = unproject_from_card(face, lo, hi, uv, depth);
            assert!(back.distance(p) < 1e-4, "{face:?}: {back:?}");
        }
    }

    #[test]
    fn face_weights_sum_to_one_and_follow_the_normal() {
        let w = face_weights(Vec3::X);
        assert_eq!(w, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let sum: f32 = face_weights(Vec3::new(1.0, 1.0, 0.0).normalize()).iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        let equal = face_weights(Vec3::ZERO);
        assert!(equal.iter().all(|&v| (v - 1.0 / 6.0).abs() < 1e-6));
    }
}
