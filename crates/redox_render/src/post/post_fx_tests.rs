//! Unit tests of the post effect maths in `post_fx.rs`.

use super::*;

const HD: [f32; 2] = [1920.0, 1080.0];

#[test]
fn a_still_pixel_gets_no_taps() {
    let s = MotionBlurSettings::default();
    assert!(motion_blur_offsets([0.0, 0.0], HD, &s).is_empty());
    // Half a pixel of velocity with the shutter at 0.5 is a quarter of a pixel: still sharp.
    assert!(motion_blur_offsets([0.5 / 1920.0, 0.0], HD, &s).is_empty());
    assert!(motion_blur_offsets([f32::NAN, 0.0], HD, &s).is_empty());
}

#[test]
fn taps_are_symmetric_and_as_many_as_requested() {
    let s = MotionBlurSettings {
        taps: 8,
        ..Default::default()
    };
    let o = motion_blur_offsets([0.02, 0.0], HD, &s);
    assert_eq!(o.len(), 8);
    for i in 0..4 {
        assert!((o[i][0] + o[7 - i][0]).abs() < 1e-7);
    }
    assert!(o[0][0] < 0.0 && o[7][0] > 0.0);
}

#[test]
fn the_blur_length_is_limited_to_max_pixels() {
    let s = MotionBlurSettings {
        shutter: 1.0,
        max_pixels: 10.0,
        taps: 5,
    };
    let o = motion_blur_offsets([0.5, 0.0], [1000.0, 1000.0], &s); // would be 500 px
    let span_px = (o[4][0] - o[0][0]) * 1000.0;
    assert!((span_px - 10.0).abs() < 1e-3, "span {span_px}");
}

#[test]
fn the_direction_follows_the_velocity() {
    let o = motion_blur_offsets(
        [0.0, 0.03],
        [1000.0, 1000.0],
        &MotionBlurSettings::default(),
    );
    assert!(!o.is_empty());
    assert!(o.iter().all(|t| t[0].abs() < 1e-9));
}

#[test]
fn the_number_of_taps_is_kept_within_limits() {
    let few = MotionBlurSettings {
        taps: 0,
        ..Default::default()
    };
    assert_eq!(
        motion_blur_offsets([0.05, 0.0], [1000.0, 1000.0], &few).len(),
        2
    );
    let many = MotionBlurSettings {
        taps: 10_000,
        ..Default::default()
    };
    let n = motion_blur_offsets([0.05, 0.0], [1000.0, 1000.0], &many).len();
    assert!((2..=64).contains(&n), "{n}");
}

#[test]
fn the_focus_plane_is_sharp() {
    assert_eq!(coc_pixels(10.0, &DofSettings::default()), 0.0);
}

#[test]
fn the_blur_grows_away_from_focus_and_is_capped() {
    let s = DofSettings {
        focus_distance: 10.0,
        aperture: 6.0,
        max_radius_pixels: 8.0,
    };
    assert!(coc_pixels(5.0, &s) > 0.0);
    assert!(coc_pixels(2.0, &s) >= coc_pixels(5.0, &s));
    assert_eq!(coc_pixels(0.5, &s), 8.0);
    assert!(coc_pixels(1.0e6, &s) <= 8.0);
}

#[test]
fn bad_distances_give_zero_blur() {
    let s = DofSettings::default();
    for d in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(coc_pixels(d, &s), 0.0, "{d}");
    }
}

#[test]
fn disc_taps_stay_inside_the_unit_disc_and_start_at_the_centre() {
    let t = disc_taps(16);
    assert_eq!(t.len(), 16);
    assert_eq!(t[0], [0.0, 0.0]);
    assert!(
        t.iter()
            .all(|p| (p[0] * p[0] + p[1] * p[1]).sqrt() <= 1.0 + 1e-6)
    );
    assert_eq!(disc_taps(0).len(), 1, "at least the centre");
}

#[test]
fn linear_distance_matches_near_and_far() {
    // Standard depth: 0 at the near plane, 1 at the far plane (the renderer's convention).
    assert!((linear_distance(0.0, 0.1, 100.0) - 0.1).abs() < 1e-4);
    assert!((linear_distance(1.0, 0.1, 100.0) - 100.0).abs() < 1e-1);
}
