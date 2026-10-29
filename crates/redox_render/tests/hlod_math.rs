//! HLOD (VG-04): proxy building, the `.hlod` file and the distance state machine. CPU only.

#![allow(clippy::unwrap_used)]

use redox_math::{Mat4, Vec3};
use redox_render::hlod::{
    CellLod, HlodConfig, HlodError, HlodInput, HlodParams, HlodProxy, build_proxy, opacities,
    step_cell_lod,
};
use redox_render::mesh::primitive::{create_cube, create_sphere};

fn params(target: usize) -> HlodParams {
    HlodParams {
        target_triangles: target,
        max_error: 10.0,
        weld_epsilon: 1e-4,
    }
}

fn input<'a>(
    positions: &'a [[f32; 3]],
    indices: &'a [u32],
    at: Vec3,
    scale: f32,
    color: [f32; 3],
) -> HlodInput<'a> {
    HlodInput {
        positions,
        indices,
        transform: Mat4::from_scale_rotation_translation(
            Vec3::splat(scale),
            redox_math::Quat::IDENTITY,
            at,
        ),
        color,
    }
}

fn cube_data() -> (Vec<[f32; 3]>, Vec<u32>) {
    let mesh = create_cube();
    (
        mesh.vertices.iter().map(|v| v.position).collect(),
        mesh.indices.clone(),
    )
}

fn sphere_data() -> (Vec<[f32; 3]>, Vec<u32>) {
    let mesh = create_sphere(1.0, 32);
    (
        mesh.vertices.iter().map(|v| v.position).collect(),
        mesh.indices.clone(),
    )
}

#[test]
fn a_proxy_has_at_most_the_target_triangle_count() {
    let (p, i) = sphere_data();
    let inputs: Vec<_> = (0..6)
        .map(|k| input(&p, &i, Vec3::new(k as f32 * 3.0, 0.0, 0.0), 1.0, [0.5; 3]))
        .collect();
    let proxy = build_proxy(&inputs, &params(300)).unwrap();
    assert!(
        proxy.indices.len() / 3 <= 300,
        "{}",
        proxy.indices.len() / 3
    );
    assert!(proxy.source_triangles > 300);
}

#[test]
fn the_proxy_is_cheaper_than_the_sum_of_the_sources() {
    let (p, i) = sphere_data();
    let inputs: Vec<_> = (0..4)
        .map(|k| input(&p, &i, Vec3::new(k as f32 * 3.0, 0.0, 0.0), 1.0, [0.5; 3]))
        .collect();
    let proxy = build_proxy(&inputs, &params(200)).unwrap();
    assert!(proxy.indices.len() < i.len() * 4);
}

#[test]
fn a_proxy_covers_the_bounds_of_its_sources() {
    let (p, i) = cube_data();
    let a = input(&p, &i, Vec3::new(-4.0, 0.0, 0.0), 1.0, [1.0, 0.0, 0.0]);
    let b = input(&p, &i, Vec3::new(4.0, 0.0, 0.0), 1.0, [0.0, 0.0, 1.0]);
    let proxy = build_proxy(&[a, b], &params(1000)).unwrap();
    let min_x = proxy.vertices.iter().map(|v| v[0]).fold(f32::MAX, f32::min);
    let max_x = proxy.vertices.iter().map(|v| v[0]).fold(f32::MIN, f32::max);
    assert!(min_x < -4.0 && max_x > 4.0, "{min_x} .. {max_x}");
}

#[test]
fn the_colour_is_the_area_weighted_mean() {
    // Two triangles share the vertex at the origin: a large red one and a small blue one.
    let big = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
    let small = [[0.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]];
    let tri = [0u32, 1, 2];
    let red = input(&big, &tri, Vec3::ZERO, 1.0, [1.0, 0.0, 0.0]);
    let blue = input(&small, &tri, Vec3::ZERO, 1.0, [0.0, 0.0, 1.0]);
    let proxy = build_proxy(&[red, blue], &params(1000)).unwrap();
    let shared = proxy
        .vertices
        .iter()
        .position(|v| v.iter().all(|c| c.abs() < 1e-3))
        .expect("the shared vertex is welded into one");
    let c = proxy.colors[shared];
    assert!(c[0] > 0.9 && c[2] < 0.1, "{c:?}");
}

#[test]
fn building_twice_gives_identical_output() {
    let (p, i) = sphere_data();
    let inputs = [input(&p, &i, Vec3::ZERO, 1.0, [0.2, 0.4, 0.6])];
    let a = build_proxy(&inputs, &params(200)).unwrap();
    let b = build_proxy(&inputs, &params(200)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn empty_input_is_an_error() {
    assert_eq!(
        build_proxy(&[], &params(100)).unwrap_err(),
        HlodError::NoInput
    );
}

#[test]
fn a_small_mesh_below_the_target_is_kept_intact() {
    let (p, i) = cube_data();
    let proxy = build_proxy(&[input(&p, &i, Vec3::ZERO, 1.0, [1.0; 3])], &params(1000)).unwrap();
    assert_eq!(proxy.indices.len() / 3, proxy.source_triangles);
}

// ---------------------------------------------------------------- the file

fn sample_proxy() -> HlodProxy {
    let (p, i) = sphere_data();
    build_proxy(
        &[input(&p, &i, Vec3::ZERO, 1.0, [0.3, 0.6, 0.9])],
        &params(150),
    )
    .unwrap()
}

#[test]
fn proxy_bytes_roundtrip() {
    let proxy = sample_proxy();
    let bytes = proxy.to_bytes((3, -7));
    let (cell, back) = HlodProxy::from_bytes(&bytes).unwrap();
    assert_eq!(cell, (3, -7));
    assert_eq!(back, proxy);
}

#[test]
fn a_truncated_or_damaged_hlod_file_is_an_error() {
    let bytes = sample_proxy().to_bytes((0, 0));
    for cut in [0usize, 3, 10, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            HlodProxy::from_bytes(&bytes[..cut]).is_err(),
            "cut at {cut}"
        );
    }
    let mut bad = bytes.clone();
    bad[0] ^= 0xFF;
    assert_eq!(
        HlodProxy::from_bytes(&bad).unwrap_err(),
        HlodError::BadMagic
    );
}

// ---------------------------------------------------------------- switching

fn run(mut state: CellLod, distance: f32, seconds: f32, cfg: &HlodConfig) -> CellLod {
    let dt = 1.0 / 60.0;
    for _ in 0..(seconds / dt) as usize {
        state = step_cell_lod(state, distance, dt, cfg);
    }
    state
}

#[test]
fn a_close_cell_stays_in_detail() {
    let cfg = HlodConfig::default();
    assert_eq!(run(CellLod::Detail, 50.0, 5.0, &cfg), CellLod::Detail);
}

#[test]
fn a_far_cell_fades_to_the_proxy_and_stays_there() {
    let cfg = HlodConfig::default();
    let mid = run(CellLod::Detail, 200.0, 0.3, &cfg);
    assert!(
        matches!(mid, CellLod::Fading { to_proxy: true, .. }),
        "{mid:?}"
    );
    assert_eq!(run(CellLod::Detail, 200.0, 3.0, &cfg), CellLod::Proxy);
}

#[test]
fn inside_the_hysteresis_band_nothing_changes() {
    let cfg = HlodConfig::default();
    assert_eq!(run(CellLod::Detail, 125.0, 2.0, &cfg), CellLod::Detail);
    assert_eq!(run(CellLod::Proxy, 115.0, 2.0, &cfg), CellLod::Proxy);
}

#[test]
fn reversing_mid_fade_does_not_jump_the_opacity() {
    let cfg = HlodConfig::default();
    let fading = run(CellLod::Detail, 200.0, 0.3, &cfg);
    let (detail_before, _) = opacities(fading);
    let reversed = step_cell_lod(fading, 50.0, 1.0 / 60.0, &cfg);
    let (detail_after, _) = opacities(reversed);
    assert!(
        (detail_after - detail_before).abs() <= 1.0 / 60.0 / cfg.fade_seconds + 1e-4,
        "{detail_before} -> {detail_after}"
    );
}

#[test]
fn the_opacities_sum_to_one_during_a_fade() {
    let cfg = HlodConfig::default();
    let mut state = CellLod::Detail;
    for _ in 0..60 {
        state = step_cell_lod(state, 200.0, 1.0 / 60.0, &cfg);
        let (d, p) = opacities(state);
        assert!((d + p - 1.0).abs() < 1e-5, "{state:?}");
    }
}

#[test]
fn nan_and_negative_inputs_do_not_change_the_state() {
    let cfg = HlodConfig::default();
    let s = CellLod::Fading {
        to_proxy: true,
        t: 0.4,
    };
    assert_eq!(step_cell_lod(s, f32::NAN, 0.016, &cfg), s);
    assert_eq!(step_cell_lod(s, 200.0, f32::NAN, &cfg), s);
    assert_eq!(step_cell_lod(s, -1.0, 0.016, &cfg), s);
}
