//! Box-shaped reflection probe math: influence weight, parallax correction and probe selection.
//! Pure functions without GPU access.

use redox_math::Vec3;

/// Box-shaped reflection probe (world space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionProbe {
    /// Capture point.
    pub center: Vec3,
    /// Half-size of the influence box around `center`.
    pub half_extent: Vec3,
    /// Distance inside the box edge over which the weight falls from 1 to 0.
    pub blend_distance: f32,
    /// Larger wins when boxes overlap.
    pub priority: i32,
}

/// Axis components below this magnitude are treated as parallel to the box face.
const PARALLEL_EPSILON: f32 = 1e-6;

fn component(v: Vec3, axis: usize) -> f32 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// Signed distance-free test: whether `local` (relative to the centre) lies inside the box.
fn inside_box(half: Vec3, local: Vec3) -> bool {
    (0..3).all(|axis| component(local, axis).abs() <= component(half, axis))
}

/// Direction after box (parallax) correction: the point where the ray from `pos` along `dir`
/// leaves the box, minus the capture centre. `dir` is expected to be normalised.
///
/// If `pos` is outside the box the direction is returned unchanged.
pub fn parallax_direction(probe: &ReflectionProbe, pos: Vec3, dir: Vec3) -> Vec3 {
    let local = pos - probe.center;
    if !inside_box(probe.half_extent, local) {
        return dir;
    }
    let mut exit_t = f32::INFINITY;
    for axis in 0..3 {
        let d = component(dir, axis);
        if d.abs() <= PARALLEL_EPSILON {
            continue;
        }
        let face = if d > 0.0 {
            component(probe.half_extent, axis)
        } else {
            -component(probe.half_extent, axis)
        };
        exit_t = exit_t.min((face - component(local, axis)) / d);
    }
    if !exit_t.is_finite() {
        return dir;
    }
    local + dir * exit_t
}

/// Weight in `[0, 1]`: 0 outside the box, 1 deeper than `blend_distance` from every face,
/// linear in between.
pub fn probe_weight(probe: &ReflectionProbe, pos: Vec3) -> f32 {
    let local = pos - probe.center;
    let mut face_distance = f32::INFINITY;
    for axis in 0..3 {
        let distance = component(probe.half_extent, axis) - component(local, axis).abs();
        face_distance = face_distance.min(distance);
    }
    if face_distance < 0.0 {
        return 0.0;
    }
    if probe.blend_distance <= 0.0 {
        return 1.0;
    }
    (face_distance / probe.blend_distance).clamp(0.0, 1.0)
}

/// Picks at most two probes (highest priority, then highest weight) and normalises their weights
/// so that they sum to `min(1, w0 + w1)`; the remainder goes to the global IBL.
///
/// Returns `(index, weight)` pairs; an unused slot is `(0, 0.0)`.
pub fn select_probes(probes: &[ReflectionProbe], pos: Vec3) -> [(usize, f32); 2] {
    let mut candidates: Vec<(usize, i32, f32)> = probes
        .iter()
        .enumerate()
        .filter_map(|(index, probe)| {
            let weight = probe_weight(probe, pos);
            (weight > 0.0).then_some((index, probe.priority, weight))
        })
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.total_cmp(&a.2)));

    let mut slots = [(0usize, 0.0f32); 2];
    for (slot, &(index, _, weight)) in slots.iter_mut().zip(candidates.iter().take(2)) {
        *slot = (index, weight);
    }
    let sum = slots[0].1 + slots[1].1;
    if sum > 1.0 {
        for slot in &mut slots {
            slot.1 /= sum;
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(center: Vec3, half: f32, blend: f32, priority: i32) -> ReflectionProbe {
        ReflectionProbe {
            center,
            half_extent: Vec3::splat(half),
            blend_distance: blend,
            priority,
        }
    }

    fn assert_vec_near(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() < 1e-4,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn the_parallax_direction_hits_the_wall_in_front() {
        let p = probe(Vec3::ZERO, 5.0, 1.0, 0);
        let hit = parallax_direction(&p, Vec3::ZERO, Vec3::X);
        assert_vec_near(hit, Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn a_shifted_position_changes_the_corrected_direction() {
        let p = probe(Vec3::ZERO, 5.0, 1.0, 0);
        let pos = Vec3::new(2.0, 0.0, 0.0);
        assert_vec_near(parallax_direction(&p, pos, Vec3::X), Vec3::new(5.0, 0.0, 0.0));
        assert_vec_near(
            parallax_direction(&p, pos, Vec3::Y),
            Vec3::new(2.0, 5.0, 0.0),
        );
    }

    #[test]
    fn outside_the_box_the_direction_is_unchanged() {
        let p = probe(Vec3::ZERO, 5.0, 1.0, 0);
        let pos = Vec3::new(9.0, 0.0, 0.0);
        assert_vec_near(parallax_direction(&p, pos, Vec3::X), Vec3::X);
    }

    #[test]
    fn the_weight_is_zero_outside_one_inside_and_linear_in_the_blend_zone() {
        let p = probe(Vec3::ZERO, 5.0, 1.0, 0);
        assert_eq!(probe_weight(&p, Vec3::new(6.0, 0.0, 0.0)), 0.0);
        assert_eq!(probe_weight(&p, Vec3::ZERO), 1.0);
        let half_way = probe_weight(&p, Vec3::new(4.5, 0.0, 0.0));
        assert!((half_way - 0.5).abs() < 1e-5, "got {half_way}");
    }

    #[test]
    fn the_higher_priority_probe_wins_when_boxes_overlap() {
        let low = probe(Vec3::ZERO, 5.0, 1.0, 0);
        let high = probe(Vec3::new(1.0, 0.0, 0.0), 5.0, 1.0, 3);
        let picked = select_probes(&[low, high], Vec3::new(0.5, 0.0, 0.0));
        assert_eq!(picked[0].0, 1);
        assert_eq!(picked[1].0, 0);
    }

    #[test]
    fn weights_of_two_probes_do_not_exceed_one() {
        let a = probe(Vec3::ZERO, 5.0, 1.0, 0);
        let b = probe(Vec3::new(1.0, 0.0, 0.0), 5.0, 1.0, 0);
        let picked = select_probes(&[a, b], Vec3::new(0.5, 0.0, 0.0));
        let sum = picked[0].1 + picked[1].1;
        assert!(sum <= 1.0 + 1e-6, "sum {sum}");
        assert!((sum - 1.0).abs() < 1e-6, "sum {sum}");
    }

    #[test]
    fn no_probe_gives_zero_weights() {
        assert_eq!(select_probes(&[], Vec3::ZERO), [(0, 0.0), (0, 0.0)]);
    }
}
