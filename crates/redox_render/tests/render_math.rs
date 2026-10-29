//! Pure CPU math of the wave-seven render features: shadow classes (RND-08), SSR, fog, SSS, refraction and
//! Substrate. No GPU needed.

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;

use redox_math::{Aabb, Mat4, Vec3};
use redox_render::post::fog::{FogSettings, hg_phase, optical_depth, slice_depth, transmittance};
use redox_render::post::refraction::{refraction_mip_level, refraction_offset};
use redox_render::post::ssr::{
    SsrSettings, edge_fade, fresnel_weight, roughness_weight, ssr_weight,
};
use redox_render::post::sss::{blur_radius_pixels, sss_kernel, wrap_diffuse};
use redox_render::shadow::classify::{
    ShadowClass, is_dynamic_caster, moved_shadow_bounds, next_shadow_poses,
};
use redox_render::substrate::{
    Coat, Slab, coat_fresnel, d_ggx, d_ggx_aniso, mix_slabs, slab_albedo, transmitted_through_coat,
    vertical_albedo,
};

// ---------------------------------------------------------------- shadow classes (RND-08)

fn bounds(size: [f32; 3]) -> Aabb {
    Aabb::from_center_size(Vec3::ZERO, Vec3::from(size))
}

#[test]
fn static_is_never_dynamic_even_when_small() {
    assert!(!is_dynamic_caster(
        ShadowClass::Static,
        &bounds([1.0, 1.0, 1.0])
    ));
}

#[test]
fn dynamic_is_dynamic_even_when_large() {
    assert!(is_dynamic_caster(
        ShadowClass::Dynamic,
        &bounds([20.0, 1.0, 20.0])
    ));
}

#[test]
fn auto_keeps_the_size_rule() {
    // `from_center_size` takes the half extents, so double the sizes below.
    assert!(!is_dynamic_caster(
        ShadowClass::Auto,
        &Aabb {
            min: Vec3::new(-10.0, 0.0, -10.0),
            max: Vec3::new(10.0, 0.3, 10.0),
        }
    ));
    assert!(is_dynamic_caster(
        ShadowClass::Auto,
        &Aabb {
            min: Vec3::splat(-0.5),
            max: Vec3::splat(0.5),
        }
    ));
    assert!(!is_dynamic_caster(
        ShadowClass::Auto,
        &Aabb {
            min: Vec3::new(-3.5, 0.0, -1.0),
            max: Vec3::new(3.5, 2.0, 1.0),
        }
    ));
}

fn pose(x: f32) -> (u32, Mat4, Aabb) {
    let at = Vec3::new(x, 0.0, 0.0);
    (
        1,
        Mat4::from_translation(at),
        Aabb {
            min: at - Vec3::splat(0.5),
            max: at + Vec3::splat(0.5),
        },
    )
}

fn snapshot(items: &[(u32, Mat4, Aabb)]) -> HashMap<u32, (Mat4, Aabb)> {
    items.iter().map(|(id, m, b)| (*id, (*m, *b))).collect()
}

#[test]
fn an_unmoved_object_invalidates_nothing() {
    let prev = snapshot(&[pose(2.0)]);
    assert!(moved_shadow_bounds(&prev, &[pose(2.0)], 0.05).is_empty());
}

#[test]
fn a_moved_object_invalidates_old_and_new_bounds() {
    let prev = snapshot(&[pose(2.0)]);
    let out = moved_shadow_bounds(&prev, &[pose(3.0)], 0.05);
    assert_eq!(out.len(), 2);
    assert!(out.iter().any(|b| b.center().x == 2.0));
    assert!(out.iter().any(|b| b.center().x == 3.0));
}

#[test]
fn slow_drift_accumulates_until_it_exceeds_the_threshold() {
    let mut prev = snapshot(&[pose(0.0)]);
    let mut invalidations = 0;
    for step in 1..=10 {
        let cur = [pose(step as f32 * 0.01)];
        if !moved_shadow_bounds(&prev, &cur, 0.05).is_empty() {
            invalidations += 1;
        }
        prev = next_shadow_poses(&prev, &cur, 0.05);
    }
    assert_eq!(invalidations, 1, "one refresh after the drift passed 5 cm");
}

#[test]
fn a_removed_object_invalidates_its_last_bounds() {
    let prev = snapshot(&[pose(2.0)]);
    let out = moved_shadow_bounds(&prev, &[], 0.05);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].center().x, 2.0);
}

#[test]
fn a_new_object_invalidates_its_current_bounds() {
    let out = moved_shadow_bounds(&HashMap::new(), &[pose(2.0)], 0.05);
    assert_eq!(out.len(), 1);
}

#[test]
fn reordering_the_list_does_not_invalidate() {
    let a = pose(2.0);
    let b = (
        2,
        Mat4::from_translation(Vec3::new(9.0, 0.0, 0.0)),
        bounds([1.0; 3]),
    );
    let prev = snapshot(&[a, b]);
    assert!(moved_shadow_bounds(&prev, &[b, a], 0.05).is_empty());
}

// ---------------------------------------------------------------- SSR (PST-05)

#[test]
fn a_mirror_gets_the_full_roughness_weight_and_a_rough_surface_none() {
    assert_eq!(roughness_weight(0.0, 0.5), 1.0);
    assert_eq!(roughness_weight(0.5, 0.5), 0.0);
    assert_eq!(roughness_weight(0.9, 0.5), 0.0);
}

#[test]
fn the_edge_fade_is_one_in_the_middle_and_zero_at_the_border() {
    assert_eq!(edge_fade([0.5, 0.5]), 1.0);
    assert_eq!(edge_fade([0.0, 0.5]), 0.0);
    assert_eq!(edge_fade([1.0, 1.0]), 0.0);
}

#[test]
fn fresnel_grows_at_grazing_angles_and_metal_is_always_strong() {
    assert!(fresnel_weight(0.05, 0.0) > fresnel_weight(1.0, 0.0));
    assert!(fresnel_weight(1.0, 1.0) > 0.9);
}

#[test]
fn the_final_ssr_weight_is_capped_and_nan_gives_zero() {
    let s = SsrSettings::default();
    assert!(ssr_weight(0.0, 1.0, 0.0, [0.5, 0.5], &s) <= 0.9);
    assert_eq!(ssr_weight(f32::NAN, 0.0, 1.0, [0.5, 0.5], &s), 0.0);
}

// ---------------------------------------------------------------- fog (PST-06)

fn uniform_fog() -> FogSettings {
    FogSettings {
        density: 0.1,
        height_falloff: 0.0,
        ..FogSettings::default()
    }
}

#[test]
fn uniform_fog_is_density_times_distance() {
    let tau = optical_depth(&uniform_fog(), 0.0, 0.0, 20.0);
    assert!((tau - 2.0).abs() < 1e-5, "{tau}");
}

#[test]
fn fog_thins_with_height() {
    let s = FogSettings::default();
    let flat = optical_depth(&s, 0.0, 0.0, 30.0);
    let rising = optical_depth(&s, 0.0, 0.5, 30.0);
    assert!(rising < flat);
}

#[test]
fn the_analytic_integral_matches_a_numeric_sum() {
    let s = FogSettings::default();
    let (y0, slope, distance) = (1.0f32, 0.3f32, 40.0f32);
    let steps = 100_000;
    let h = distance / steps as f32;
    let mut sum = 0.0f64;
    for i in 0..steps {
        let t = (i as f32 + 0.5) * h;
        let y = y0 + slope * t;
        sum += f64::from(s.density * (-s.height_falloff * (y - s.base_height)).exp() * h);
    }
    let tau = optical_depth(&s, y0, slope, distance);
    assert!((f64::from(tau) - sum).abs() / sum < 1e-3, "{tau} vs {sum}");
}

#[test]
fn a_near_horizontal_ray_does_not_divide_by_zero() {
    let s = FogSettings::default();
    let tau = optical_depth(&s, 0.0, 1e-7, 25.0);
    assert!(tau.is_finite());
    assert!((tau - optical_depth(&s, 0.0, 0.0, 25.0)).abs() < 1e-3);
}

#[test]
fn transmittance_is_one_without_fog_and_falls_with_depth() {
    assert_eq!(transmittance(0.0), 1.0);
    assert!(transmittance(2.0) < transmittance(1.0));
}

#[test]
fn hg_phase_with_g_zero_is_isotropic() {
    for c in [-1.0f32, -0.3, 0.0, 0.6, 1.0] {
        assert!((hg_phase(c, 0.0) - 1.0 / (4.0 * std::f32::consts::PI)).abs() < 1e-5);
    }
}

#[test]
fn hg_phase_is_forward_peaked_for_positive_g_and_integrates_to_about_one() {
    assert!(hg_phase(1.0, 0.6) > hg_phase(-1.0, 0.6));
    let n = 4000;
    let mut sum = 0.0f64;
    for i in 0..n {
        let c = -1.0 + 2.0 * (i as f32 + 0.5) / n as f32;
        sum += f64::from(hg_phase(c, 0.5)) * 2.0 / f64::from(n);
    }
    let total = sum * 2.0 * std::f64::consts::PI;
    assert!((total - 1.0).abs() < 0.02, "{total}");
}

#[test]
fn start_distance_removes_fog_close_to_the_camera() {
    let s = FogSettings {
        start_distance: 10.0,
        ..uniform_fog()
    };
    assert_eq!(optical_depth(&s, 0.0, 0.0, 8.0), 0.0);
    assert!(optical_depth(&s, 0.0, 0.0, 20.0) > 0.0);
}

#[test]
fn slice_depths_are_increasing_and_hit_the_ends() {
    assert!((slice_depth(0, 64, 0.5, 100.0) - 0.5).abs() < 1e-5);
    assert!((slice_depth(64, 64, 0.5, 100.0) - 100.0).abs() < 1e-2);
    for i in 0..64 {
        assert!(slice_depth(i + 1, 64, 0.5, 100.0) > slice_depth(i, 64, 0.5, 100.0));
    }
}

// ---------------------------------------------------------------- SSS (MAT-05) and refraction (MAT-06)

#[test]
fn zero_wrap_equals_the_lambert_term_and_full_light_stays_one() {
    for x in [-0.5f32, 0.0, 0.3, 1.0] {
        assert!((wrap_diffuse(x, 0.0) - x.max(0.0)).abs() < 1e-6);
    }
    assert!((wrap_diffuse(1.0, 0.7) - 1.0).abs() < 1e-6);
}

#[test]
fn wrap_lights_the_shadow_side_a_little_and_is_monotone() {
    assert!(wrap_diffuse(-0.2, 0.5) > 0.0);
    let mut last = -1.0;
    for i in 0..=20 {
        let v = wrap_diffuse(-1.0 + i as f32 * 0.1, 0.5);
        assert!(v >= last);
        last = v;
    }
}

#[test]
fn kernel_weights_sum_to_one_per_channel_and_are_symmetric() {
    for taps in [7, 9, 11] {
        let k = sss_kernel([0.9, 0.4, 0.2], taps);
        assert_eq!(k.len(), taps);
        for c in 0..3 {
            let sum: f32 = k.iter().map(|s| s.weight[c]).sum();
            assert!((sum - 1.0).abs() < 1e-4, "taps {taps} channel {c}: {sum}");
        }
        for i in 0..taps / 2 {
            let (a, b) = (k[i], k[taps - 1 - i]);
            assert!((a.offset + b.offset).abs() < 1e-5);
            for c in 0..3 {
                assert!((a.weight[c] - b.weight[c]).abs() < 1e-5);
            }
        }
    }
}

#[test]
fn the_kernel_centre_is_the_heaviest_and_red_scatters_wider_than_blue() {
    let k = sss_kernel([0.9, 0.4, 0.2], 9);
    let centre = k.len() / 2;
    for s in &k {
        assert!(k[centre].weight[0] >= s.weight[0] - 1e-6);
    }
    let spread = |c: usize| k.iter().map(|s| s.offset.abs() * s.weight[c]).sum::<f32>();
    assert!(spread(0) > spread(2));
}

#[test]
fn the_blur_radius_falls_with_distance_and_grows_with_radius() {
    let near = blur_radius_pixels(5.0, 1.0, 1.0, 1080.0);
    let far = blur_radius_pixels(5.0, 4.0, 1.0, 1080.0);
    assert!(near > far);
    assert!(blur_radius_pixels(10.0, 1.0, 1.0, 1080.0) > near);
    assert_eq!(blur_radius_pixels(0.0, 1.0, 1.0, 1080.0), 0.0);
    assert!(blur_radius_pixels(5.0, 0.0, 0.0, 1080.0).is_finite());
}

#[test]
fn refraction_offset_is_zero_at_ior_one_and_grows_with_tilt() {
    assert_eq!(refraction_offset(1.0, [0.5, 0.5], 1.0, 2.0), [0.0, 0.0]);
    let small = refraction_offset(1.5, [0.1, 0.0], 1.0, 2.0);
    let large = refraction_offset(1.5, [0.5, 0.0], 1.0, 2.0);
    assert!(large[0].abs() > small[0].abs());
    let far = refraction_offset(1.5, [0.5, 0.0], 1.0, 10.0);
    assert!(far[0].abs() < large[0].abs());
}

#[test]
fn mip_level_scales_with_roughness_and_is_clamped() {
    assert_eq!(refraction_mip_level(0.0, 6), 0.0);
    assert!(refraction_mip_level(0.5, 6) > 0.0);
    assert!(refraction_mip_level(5.0, 6) <= 5.0);
}

// ---------------------------------------------------------------- Substrate (MAT-04)

#[test]
fn a_slab_never_reflects_more_than_it_receives() {
    for roughness in [0.05f32, 0.2, 0.35, 0.5, 0.65, 0.8, 1.0] {
        for metallic in [0.0f32, 0.5, 1.0] {
            for n_dot_v in [0.05f32, 0.3, 0.6, 1.0] {
                let slab = Slab {
                    roughness,
                    metallic,
                    ..Slab::default()
                };
                let a = slab_albedo(&slab, n_dot_v, 64);
                assert!(a <= 1.0 + 1e-3, "{roughness} {metallic} {n_dot_v}: {a}");
            }
        }
    }
}

#[test]
fn a_coated_slab_never_reflects_more_than_it_receives() {
    for weight in [0.0f32, 0.5, 1.0] {
        for coat_roughness in [0.05f32, 0.4, 1.0] {
            for ior in [1.3f32, 1.5, 2.0] {
                for n_dot_v in [0.1f32, 0.5, 1.0] {
                    let coat = Coat {
                        weight,
                        roughness: coat_roughness,
                        ior,
                        ..Coat::default()
                    };
                    let a = vertical_albedo(&coat, &Slab::default(), n_dot_v, 64);
                    assert!(
                        a <= 1.0 + 1e-2,
                        "{weight} {coat_roughness} {ior} {n_dot_v}: {a}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_zero_weight_coat_is_invisible() {
    let coat = Coat {
        weight: 0.0,
        ..Coat::default()
    };
    let slab = Slab::default();
    let a = vertical_albedo(&coat, &slab, 0.7, 64);
    assert!((a - slab_albedo(&slab, 0.7, 64)).abs() < 1e-5);
}

#[test]
fn the_coat_darkens_the_base_by_its_fresnel_at_normal_incidence() {
    let coat = Coat {
        weight: 1.0,
        ..Coat::default()
    };
    let t = transmitted_through_coat(&coat, 1.0);
    assert!((t[0] - (1.0 - 0.04)).abs() < 0.01, "{t:?}");
}

#[test]
fn coat_fresnel_grows_towards_grazing_angles() {
    let coat = Coat {
        weight: 1.0,
        ..Coat::default()
    };
    assert!(coat_fresnel(&coat, 0.05) > coat_fresnel(&coat, 1.0));
}

#[test]
fn extreme_roughness_values_stay_finite() {
    for roughness in [0.0f32, 1e-6, 1.0] {
        assert!(d_ggx(0.9, roughness).is_finite());
        assert!(
            slab_albedo(
                &Slab {
                    roughness,
                    ..Slab::default()
                },
                0.5,
                32
            )
            .is_finite()
        );
    }
}

#[test]
fn zero_anisotropy_equals_isotropic_ggx() {
    for n_dot_h in [0.5f32, 0.8, 0.99] {
        let h_dot_t = (1.0 - n_dot_h * n_dot_h).sqrt() * 0.6;
        let h_dot_b = (1.0 - n_dot_h * n_dot_h).sqrt() * 0.8;
        let iso = d_ggx(n_dot_h, 0.4);
        let aniso = d_ggx_aniso(n_dot_h, h_dot_t, h_dot_b, 0.4, 0.0);
        assert!((iso - aniso).abs() / iso < 1e-3, "{iso} vs {aniso}");
    }
}

#[test]
fn mixing_slabs_with_mask_zero_and_one_returns_the_ends() {
    let a = Slab {
        roughness: 0.2,
        ..Slab::default()
    };
    let b = Slab {
        roughness: 0.9,
        metallic: 1.0,
        ..Slab::default()
    };
    assert_eq!(mix_slabs(&a, &b, 0.0), a);
    assert_eq!(mix_slabs(&a, &b, 1.0), b);
    let mid = mix_slabs(&a, &b, 0.5);
    assert!((mid.roughness - 0.55).abs() < 1e-6);
}
