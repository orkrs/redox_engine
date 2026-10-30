//! GI-06 step 4: streamed SDF bricks reach the GI volume when the camera crosses a cell border.
//! The streaming logic is `redox_core::gi_streaming`; the frames come from the whole renderer.
//! Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::PathBuf;
use std::time::Duration;

use redox_core::gi_streaming::{
    GI_SDF_BRICK_COLOUR, GiSdfStreamer, GiSdfStreaming, GiStreamingSettings, gi_sdf_streaming_step,
};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::sdf::atlas::SdfBrick;
use redox_render::sdf::bake::bake_mesh;
use redox_render::sdf::format::write_bricks;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
/// Frames drawn per view; the volume rebuild runs on a worker thread.
const FRAMES: u32 = 120;
/// Side of a world cell, metres.
const CELL: f32 = 64.0;
/// Side of a brick's domain cube, metres.
const BRICK_WORLD: f32 = 4.0;

/// Only the cell the camera is in is in range: radius 40 m.
fn settings() -> GiStreamingSettings {
    GiStreamingSettings {
        radius: 40.0,
        hysteresis: 16.0,
        max_bricks: 4096,
        min_interval: 0.0,
    }
}

/// Centre of cell `cell`, on the floor.
fn cell_centre(cell: (i32, i32)) -> Vec3 {
    Vec3::new(
        (cell.0 as f32 + 0.5) * CELL,
        0.5,
        (cell.1 as f32 + 0.5) * CELL,
    )
}

/// Brick of a unit cube centred on `centre`.
fn cube_brick(centre: Vec3) -> SdfBrick {
    SdfBrick {
        field: bake_mesh(&create_cube(), Vec3::splat(BRICK_WORLD)),
        origin_ws: centre,
        world_size: Vec3::splat(BRICK_WORLD),
    }
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("redox_gi_sdf_stream_{}", std::process::id()))
}

/// One unit cube per cell centre, visible to the camera and lit by the scene light.
fn cube_objects(renderer: &mut Renderer, centres: &[Vec3]) -> Vec<RenderObject> {
    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::splat(0.8)));
    centres
        .iter()
        .enumerate()
        .map(|(i, c)| RenderObject {
            id: i as u32,
            shadow_class: ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_translation(*c),
            color: [1.0; 4],
            mesh_index: mesh,
            material_index: material,
            bounds: Aabb {
                min: *c - Vec3::splat(0.5),
                max: *c + Vec3::splat(0.5),
            },
        })
        .collect()
}

fn scene_lights() -> LightScene {
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, -1.0, -0.3),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.2);
    lights
}

/// Hands the bricks of one streaming step to the renderer, with the game's colour.
fn apply(renderer: &mut Renderer, bricks: Vec<SdfBrick>) {
    let pairs: Vec<(SdfBrick, [f32; 4])> = bricks
        .into_iter()
        .map(|brick| (brick, GI_SDF_BRICK_COLOUR))
        .collect();
    renderer.set_gi_static_bricks(&pairs);
}

/// Draws `frames` frames from `eye` looking down -Z; returns the last as RGBA bytes.
fn render(renderer: &mut Renderer, objects: &[RenderObject], eye: Vec3, frames: u32) -> Vec<u8> {
    let camera = Camera::new(1.2, 1.0, 0.05, 60.0);
    renderer.camera_uniform.update(&camera, eye, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let lights = scene_lights();
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gi_sdf_stream_target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for _ in 0..frames {
        let fv = FrameView::new(&camera, objects, &lights);
        renderer.render_frame_view(&fv, &view).expect("frame");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        std::thread::sleep(Duration::from_millis(5));
    }
    read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)
        .unwrap()
        .levels
        .swap_remove(0)
}

/// Mean brightness of all pixels (average of the RGB channels), 0..255.
fn mean_luma(frame: &[u8]) -> f64 {
    let sum: f64 = frame
        .chunks_exact(4)
        .map(|px| f64::from(px[0]) + f64::from(px[1]) + f64::from(px[2]))
        .sum();
    sum / (frame.len() / 4 * 3) as f64
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

#[test]
fn moving_the_camera_across_a_cell_border_updates_the_probe_lighting() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    let dir = temp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    for cell in [(0, 0), (1, 0)] {
        let brick = cube_brick(cell_centre(cell));
        std::fs::write(
            dir.join(format!("{}_{}.sdf", cell.0, cell.1)),
            write_bricks(&[brick]),
        )
        .unwrap();
    }
    let objects = cube_objects(&mut renderer, &[cell_centre((0, 0)), cell_centre((1, 0))]);
    let mut res = GiSdfStreaming {
        dir: dir.clone(),
        cell_size: CELL,
        available: vec![(0, 0), (1, 0)],
        settings: settings(),
        streamer: GiSdfStreamer::new(),
    };

    // Camera in cell (0, 0): only that cell's brick is streamed in.
    let eye_a = Vec3::new(32.0, 1.2, 36.0);
    let bricks = gi_sdf_streaming_step(&mut res, 1.0, eye_a).expect("first step loads");
    assert_eq!(bricks.len(), 1);
    apply(&mut renderer, bricks);
    let frame_a = render(&mut renderer, &objects, eye_a, FRAMES);
    assert!(
        renderer.gi_probes.enabled(),
        "probes must be on after the bricks"
    );
    assert!(mean_luma(&frame_a) > 1.0, "frame in cell (0, 0) is empty");

    // Camera crosses into cell (1, 0): the window slides to that brick.
    let eye_b = Vec3::new(96.0, 1.2, 36.0);
    let bricks = gi_sdf_streaming_step(&mut res, 1.0, eye_b).expect("border crossing reloads");
    assert_eq!(bricks.len(), 1);
    assert_eq!(bricks[0].origin_ws.x, 96.0);
    apply(&mut renderer, bricks);

    // Right after the switch the frame must still be drawn, not blank.
    let just_after = render(&mut renderer, &objects, eye_b, 1);
    assert!(
        mean_luma(&just_after) > 1.0,
        "frame right after the cell switch is empty"
    );
    let frame_b = render(&mut renderer, &objects, eye_b, FRAMES);

    // The probe lighting is in the frame: GI off gives a different image.
    renderer.set_gi_enabled(false);
    let frame_off = render(&mut renderer, &objects, eye_b, 1);
    let diff = max_diff(&frame_b, &frame_off);
    assert!(diff > 2, "GI made no difference to the frame (diff {diff})");

    std::fs::remove_dir_all(&dir).ok();
}
