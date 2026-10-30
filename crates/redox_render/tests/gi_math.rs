//! CPU-only GI tests: octahedral mapping, analytic SDF, reference path tracer, probe cascade,
//! and the global SDF volume. No GPU adapter is needed.

use redox_math::{Aabb, Vec2, Vec3};
use redox_render::gi::cascade::ProbeCascade;
use redox_render::gi::oct::{fibonacci_sphere, oct_decode, oct_encode};
use redox_render::gi::reference::reference_irradiance;
use redox_render::gi::sdf_ref::{RefScene, cornell_box, sdf_box, sdf_scene, sphere_trace};
use redox_render::sdf::SdfBrick;
use redox_render::sdf::bake::DistanceField64;
use redox_render::sdf::global::{GlobalSdf, MAX_DISTANCE, SdfBox};

/// The Cornell room with every non-emitting box re-coloured white.
fn white_room() -> RefScene {
    let mut scene = cornell_box();
    for b in &mut scene.boxes {
        if b.emissive == Vec3::ZERO {
            b.albedo = Vec3::splat(0.75);
        }
    }
    scene
}

fn room_bounds() -> Aabb {
    Aabb {
        min: Vec3::new(-2.2, -0.2, -2.2),
        max: Vec3::new(2.2, 4.2, 2.2),
    }
}

fn sum(v: Vec3) -> f32 {
    v.x + v.y + v.z
}

/// Brick holding the exact SDF of a sphere, centred on `center`, domain side `size`.
fn sphere_brick(center: Vec3, radius: f32, size: f32) -> SdfBrick {
    const RES: usize = 64;
    let mut data = vec![0.0f32; RES * RES * RES];
    for z in 0..RES {
        for y in 0..RES {
            for x in 0..RES {
                let local = (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5))
                    / RES as f32
                    * size
                    - Vec3::splat(size * 0.5);
                data[DistanceField64::index(x, y, z)] = local.length() - radius;
            }
        }
    }
    SdfBrick {
        field: DistanceField64 {
            data,
            world_size: Vec3::splat(size),
        },
        origin_ws: center,
        world_size: Vec3::splat(size),
    }
}

#[test]
fn oct_encode_then_decode_returns_the_direction() {
    for i in 0..1000 {
        let d = fibonacci_sphere(i, 1000);
        let back = oct_decode(oct_encode(d));
        assert!(
            (back - d).length() < 1e-4,
            "direction {i}: {d:?} decoded to {back:?}"
        );
    }
}

#[test]
fn oct_decode_covers_the_sphere_uniformly_enough() {
    let n = 200;
    let mut counts = [0usize; 8];
    for j in 0..n {
        for i in 0..n {
            let uv = Vec2::new((i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32);
            let d = oct_decode(uv);
            let octant = (d.x > 0.0) as usize
                | (((d.y > 0.0) as usize) << 1)
                | (((d.z > 0.0) as usize) << 2);
            counts[octant] += 1;
        }
    }
    let total = (n * n) as f32;
    for (octant, c) in counts.iter().enumerate() {
        let share = *c as f32 / total;
        assert!(
            (0.10..=0.15).contains(&share),
            "octant {octant} gets {share:.3} of the square"
        );
    }
}

#[test]
fn sdf_box_is_negative_inside_and_exact_outside() {
    let b = SdfBox {
        min: Vec3::splat(-1.0),
        max: Vec3::ONE,
        albedo: Vec3::ZERO,
        emissive: Vec3::ZERO,
    };
    assert!(sdf_box(Vec3::ZERO, &b) < 0.0);
    assert!((sdf_box(Vec3::new(2.0, 0.0, 0.0), &b) - 1.0).abs() < 1e-6);
    assert!((sdf_box(Vec3::new(2.0, 2.0, 0.0), &b) - 2f32.sqrt()).abs() < 1e-6);
}

#[test]
fn sphere_trace_hits_the_wall_of_the_cornell_box() {
    // Room centre (0, 2, 0); the right wall's inner face is at x = 2.0.
    let hit = sphere_trace(&cornell_box(), Vec3::new(0.0, 2.0, 0.0), Vec3::X, 50.0)
        .expect("ray along +X must hit the right wall");
    assert!((hit.t - 2.0).abs() < 2e-3, "t = {}", hit.t);
    assert!(hit.normal.x > 0.99, "normal = {:?}", hit.normal);
}

#[test]
fn a_ray_leaving_through_no_opening_never_escapes() {
    let scene = cornell_box();
    let centre = Vec3::new(0.0, 2.0, 0.0);
    for i in 0..256 {
        let dir = fibonacci_sphere(i, 256);
        assert!(
            sphere_trace(&scene, centre, dir, 50.0).is_some(),
            "direction {i} escaped the room"
        );
    }
}

#[test]
fn the_reference_floor_next_to_the_red_wall_is_redder_than_greener() {
    // Floor point 0.3 m from the red left wall (x = -2).
    let scene = cornell_box();
    let e = reference_irradiance(&scene, Vec3::new(-1.7, 0.0, 0.0), Vec3::Y, 3, 4096, 7);
    assert!(e.x > 1.3 * e.y, "R {} vs G {}", e.x, e.y);
    assert!(e.x > 1.3 * e.z, "R {} vs B {}", e.x, e.z);
}

#[test]
fn the_reference_is_symmetric_for_a_symmetric_scene() {
    let scene = white_room();
    let left = reference_irradiance(&scene, Vec3::new(-1.0, 0.0, 0.0), Vec3::Y, 3, 4096, 1);
    let right = reference_irradiance(&scene, Vec3::new(1.0, 0.0, 0.0), Vec3::Y, 3, 4096, 2);
    let (sl, sr) = (sum(left), sum(right));
    assert!(
        (sl - sr).abs() <= 0.05 * sl.max(sr),
        "left {sl} vs right {sr}"
    );
}

#[test]
fn more_bounces_never_darken_a_closed_room() {
    // Same seed per point: each longer path extends the shorter one, so the sums are ordered.
    let scene = cornell_box();
    let mut one = 0.0;
    let mut three = 0.0;
    for i in 0..256u32 {
        let pos = Vec3::new(
            ((i % 16) as f32 - 7.5) * 0.2,
            0.0,
            ((i / 16) as f32 - 7.5) * 0.2,
        );
        one += sum(reference_irradiance(&scene, pos, Vec3::Y, 1, 256, i));
        three += sum(reference_irradiance(&scene, pos, Vec3::Y, 3, 256, i));
    }
    assert!(three >= one, "1 bounce {one} > 3 bounces {three}");
}

#[test]
fn the_cascade_snaps_in_whole_cells_and_wraps_indices() {
    let mut cascade = ProbeCascade::new(Vec3::ZERO, 1.0, [8, 4, 8]);
    cascade.snap_to_camera(Vec3::ZERO);
    let shift = cascade.snap_to_camera(Vec3::new(3.4, 0.0, 0.0));
    assert_eq!(shift, [3, 0, 0]);

    let cell = [5, 1, 2];
    assert_eq!(
        cascade.storage_index(cell),
        cascade.storage_index([cell[0] + 8, cell[1], cell[2]])
    );
    assert_eq!(
        cascade.storage_index([-3, 1, 2]),
        cascade.storage_index(cell)
    );

    // Every probe slot maps back to a world position inside the current window.
    for slot in 0..cascade.probe_count() {
        let pos = cascade.probe_world_pos(slot);
        let window_min = Vec3::new(
            cascade.scroll[0] as f32,
            cascade.scroll[1] as f32,
            cascade.scroll[2] as f32,
        );
        let window_max = window_min + Vec3::new(8.0, 4.0, 8.0);
        assert!(
            pos.cmpge(window_min).all() && pos.cmplt(window_max).all(),
            "slot {slot} at {pos:?} outside window"
        );
        assert_eq!(
            cascade.storage_index(pos.to_array().map(|v| v as i32)),
            slot
        );
    }
}

#[test]
fn from_boxes_matches_the_analytic_sdf_at_voxel_centres() {
    let scene = cornell_box();
    let dims = [64, 64, 64];
    let sdf = GlobalSdf::from_boxes(&scene.boxes, room_bounds(), dims);
    for z in (0..dims[2]).step_by(7) {
        for y in (0..dims[1]).step_by(7) {
            for x in (0..dims[0]).step_by(7) {
                let p = sdf.voxel_centre(x, y, z);
                let exact = sdf_scene(&scene, p).0.clamp(-MAX_DISTANCE, MAX_DISTANCE);
                let got = sdf.distance[GlobalSdf::index(dims, x, y, z)];
                assert!(
                    (got - exact).abs() <= sdf.voxel * 0.5,
                    "voxel ({x},{y},{z}): {got} vs {exact}"
                );
            }
        }
    }
}

#[test]
fn from_bricks_agrees_with_the_brick_inside_its_domain() {
    let brick = sphere_brick(Vec3::new(0.0, 1.0, 0.0), 0.5, 2.0);
    let bounds = Aabb {
        min: Vec3::splat(-4.0),
        max: Vec3::splat(4.0),
    };
    let dims = [64, 64, 64];
    let sdf = GlobalSdf::from_bricks(&[(brick.clone(), [0.2, 0.4, 0.8, 0.0])], bounds, dims);
    for z in (0..dims[2]).step_by(5) {
        for y in (0..dims[1]).step_by(5) {
            for x in (0..dims[0]).step_by(5) {
                let p = sdf.voxel_centre(x, y, z);
                if !brick.contains(p) {
                    continue;
                }
                let expected = brick.sample_world(p).clamp(-MAX_DISTANCE, MAX_DISTANCE);
                let got = sdf.distance[GlobalSdf::index(dims, x, y, z)];
                assert!(
                    (got - expected).abs() <= sdf.voxel,
                    "voxel ({x},{y},{z}): {got} vs {expected}"
                );
            }
        }
    }
}

#[test]
fn outside_every_brick_the_distance_is_clamped() {
    let brick = sphere_brick(Vec3::ZERO, 0.5, 2.0);
    let bounds = Aabb {
        min: Vec3::splat(-20.0),
        max: Vec3::splat(20.0),
    };
    let sdf = GlobalSdf::from_bricks(&[(brick, [1.0, 1.0, 1.0, 0.0])], bounds, [64, 64, 64]);
    assert_eq!(sdf.sample(Vec3::new(10.0, 0.0, 0.0)), MAX_DISTANCE);
    let near = sdf.sample(Vec3::new(1.6, 0.0, 0.0));
    assert!(near > 0.0 && near < MAX_DISTANCE, "near = {near}");
}

#[test]
fn the_trilinear_sample_is_continuous_across_voxel_boundaries() {
    let scene = cornell_box();
    let sdf = GlobalSdf::from_boxes(&scene.boxes, room_bounds(), [64, 64, 64]);
    let boundary = sdf.origin + Vec3::new(10.0, 10.0, 10.0) * sdf.voxel;
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let below = sdf.sample(boundary - axis * 1e-5);
        let above = sdf.sample(boundary + axis * 1e-5);
        assert!(
            (below - above).abs() < 1e-3,
            "jump {}",
            (below - above).abs()
        );
    }
}

#[test]
fn f16_encoding_is_accurate_to_a_tenth_of_a_voxel() {
    let scene = cornell_box();
    let sdf = GlobalSdf::from_boxes(&scene.boxes, room_bounds(), [32, 32, 32]);
    let bits = sdf.distance_f16_bits();
    for (i, (&bit, &d)) in bits.iter().zip(&sdf.distance).enumerate().step_by(97) {
        let back = half::f16::from_bits(bit).to_f32();
        assert!(
            (back - d).abs() < 0.1 * sdf.voxel,
            "texel {i}: {d} -> {back}"
        );
    }
    for k in 0..1000 {
        let v = -8.0 + 16.0 * k as f32 / 999.0;
        let back = half::f16::from_f32(v).to_f32();
        assert!((back - v).abs() < 0.005, "{v} -> {back}");
    }
}
