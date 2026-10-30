//! GI scene tests (GI-01 step 5a): the Cornell room and a two-room light-leak scene, rendered
//! through the whole renderer with GI as the only light source. Skipped without a GPU.
//!
//! The lighting is indirect only: the directional light has zero intensity and the ambient is
//! zero, so the frame shows what the probe cascades deliver.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use redox_asset::{AssetManager, Handle};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::asset_types::{MaterialData, MeshData};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::gi::sdf_ref::cornell_box;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::sdf::global::SdfBox;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
/// Frames drawn before the frame is read; the first frames wait for the volume rebuild.
const FRAMES: u32 = 60;

/// Fractional rectangle of the frame: `(x0, y0, x1, y1)` with origin at the top left.
type Rect = (f32, f32, f32, f32);

/// Mean RGB of the pixels inside `rect`, in 0..255.
fn mean_rgb(frame: &[u8], rect: Rect) -> [f64; 3] {
    let size = SIZE as usize;
    let x0 = (rect.0 * SIZE as f32) as usize;
    let x1 = (rect.2 * SIZE as f32) as usize;
    let y0 = (rect.1 * SIZE as f32) as usize;
    let y1 = (rect.3 * SIZE as f32) as usize;
    let mut sum = [0.0f64; 3];
    let mut count = 0.0f64;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * size + x) * 4;
            for (c, acc) in sum.iter_mut().enumerate() {
                *acc += f64::from(frame[i + c]);
            }
            count += 1.0;
        }
    }
    sum.map(|s| s / count.max(1.0))
}

/// Mean brightness (average of the channels) of `rect`.
fn mean_luma(frame: &[u8], rect: Rect) -> f64 {
    let rgb = mean_rgb(frame, rect);
    (rgb[0] + rgb[1] + rgb[2]) / 3.0
}

/// One unit cube per box, scaled and placed to cover it, each with its own albedo material.
fn scene_objects(
    renderer: &mut Renderer,
    assets: &mut AssetManager,
    boxes: &[SdfBox],
) -> Vec<RenderObject> {
    let mesh: Handle<MeshData> = assets.insert(create_cube());
    let mesh_index = renderer
        .add_mesh_from_asset(mesh, assets.get(mesh).unwrap())
        .unwrap();
    boxes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let material = assets.insert(MaterialData::solid(b.albedo));
            let material_index = renderer
                .add_material_from_asset(material, assets.get(material).unwrap())
                .unwrap();
            let center = (b.min + b.max) * 0.5;
            let size = b.max - b.min;
            RenderObject {
                id: i as u32,
                shadow_class: ShadowClass::Auto,
                deforms: false,
                pose_version: 0,
                model_matrix: Mat4::from_translation(center) * Mat4::from_scale(size),
                color: [1.0; 4],
                mesh_index,
                material_index,
                bounds: Aabb {
                    min: b.min,
                    max: b.max,
                },
            }
        })
        .collect()
}

/// Light scene without direct light: GI is the only source.
fn indirect_only_lights() -> LightScene {
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::ONE,
        0.0,
    ));
    lights.ambient = Vec3::ZERO;
    lights
}

/// Places the camera and draws `FRAMES` frames; returns the last as RGBA bytes.
fn render_view(
    renderer: &mut Renderer,
    objects: &[RenderObject],
    eye: Vec3,
    rotation: Quat,
) -> Vec<u8> {
    let camera = Camera::new(1.2, 1.0, 0.05, 30.0);
    renderer.camera_uniform.update(&camera, eye, rotation);
    renderer.update_camera_buffer();
    let lights = indirect_only_lights();
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gi_scene_target"),
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
    for _ in 0..FRAMES {
        let fv = FrameView::new(&camera, objects, &lights);
        renderer.render_frame_view(&fv, &view).expect("frame");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        // The volume rebuild runs on a worker thread; give it time to finish.
        std::thread::sleep(Duration::from_millis(5));
    }
    read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)
        .unwrap()
        .levels
        .swap_remove(0)
}

#[test]
fn the_white_floor_next_to_the_red_wall_is_tinted_red() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    let boxes = cornell_box().boxes;
    renderer.set_gi_static_geometry(&boxes);
    let mut assets = AssetManager::new(".");
    let objects = scene_objects(&mut renderer, &mut assets, &boxes);

    // Standing in the middle of the room, looking down at the floor.
    let frame = render_view(
        &mut renderer,
        &objects,
        Vec3::new(0.0, 1.0, 0.0),
        Quat::from_rotation_x(-0.6),
    );

    // Camera-left is -X: the red wall. Camera-right is the green wall.
    let by_red = mean_rgb(&frame, (0.05, 0.75, 0.25, 0.95));
    let by_green = mean_rgb(&frame, (0.75, 0.75, 0.95, 0.95));
    assert!(
        by_red[0] > 1.2 * by_red[1],
        "floor by the red wall: R {:.1} should exceed 1.2 x G {:.1}",
        by_red[0],
        by_red[1]
    );
    assert!(
        by_green[1] > 1.2 * by_green[0],
        "floor by the green wall: G {:.1} should exceed 1.2 x R {:.1}",
        by_green[1],
        by_green[0]
    );
}

#[test]
fn light_does_not_leak_through_a_thin_wall() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    // Two rooms on one floor, divided by a 0.2 m wall taller than the emitter. The only emitter
    // is in the left room; the camera looks straight down over both rooms.
    let boxes = vec![
        SdfBox {
            min: Vec3::new(-4.0, -0.2, -2.0),
            max: Vec3::new(4.0, 0.0, 2.0),
            albedo: Vec3::splat(0.75),
            emissive: Vec3::ZERO,
        },
        SdfBox {
            min: Vec3::new(-0.1, 0.0, -2.0),
            max: Vec3::new(0.1, 3.0, 2.0),
            albedo: Vec3::splat(0.75),
            emissive: Vec3::ZERO,
        },
        SdfBox {
            min: Vec3::new(-3.0, 0.9, -1.0),
            max: Vec3::new(-2.0, 1.1, 0.0),
            albedo: Vec3::ZERO,
            emissive: Vec3::splat(12.0),
        },
    ];
    renderer.set_gi_static_geometry(&boxes);
    let mut assets = AssetManager::new(".");
    let objects = scene_objects(&mut renderer, &mut assets, &boxes);

    let frame = render_view(
        &mut renderer,
        &objects,
        Vec3::new(0.0, 1.8, 1.9),
        Quat::from_rotation_x(-0.9),
    );

    // Which image half shows the lit room depends on the camera basis; compare the two halves.
    let half_a = mean_luma(&frame, (0.1, 0.3, 0.45, 0.7));
    let half_b = mean_luma(&frame, (0.55, 0.3, 0.9, 0.7));
    let (lit, dark) = if half_a >= half_b {
        (half_a, half_b)
    } else {
        (half_b, half_a)
    };
    assert!(lit > 0.5, "the lit room is too dark to compare: {lit:.2}");
    assert!(
        dark < 0.05 * lit,
        "light leaks through the wall: dark room {dark:.2} vs lit room {lit:.2}"
    );
}
