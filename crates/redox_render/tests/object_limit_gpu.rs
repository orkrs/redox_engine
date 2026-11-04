//! FOL-01 step 1: a frame with more objects than the model buffers hold is cut, not a panic.
//! Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
/// Buffer capacity of the renderer (`renderer::init::MAX_OBJECTS_PER_FRAME`).
const CAPACITY: usize = 10_000;
/// Objects past the capacity: they must be dropped, not drawn.
const EXTRA: usize = 500;

fn cube_at(mesh: usize, center: Vec3, scale: f32, color: [f32; 4], id: u32) -> RenderObject {
    let scale = Vec3::splat(scale);
    RenderObject {
        id,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
        color,
        mesh_index: mesh,
        material_index: 0,
        bounds: Aabb::from_center_size(center, scale),
    }
}

#[test]
fn a_frame_with_more_objects_than_the_buffer_holds_does_not_panic() {
    let Some(adapter) = common::adapter() else {
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    let mesh = renderer.upload_mesh(&create_cube());
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("object_limit_target"),
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
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.4);

    // Object 0 is a red cube in the centre of the frame: the first object, always kept.
    let red = [1.0, 0.0, 0.0, 1.0];
    // Filler objects far off screen (kept, but invisible) fill up to the capacity.
    let off_screen = Vec3::new(1000.0, 0.0, -10.0);
    let mut objects = vec![cube_at(mesh, Vec3::new(0.0, 0.0, -10.0), 0.5, red, 0)];
    objects.extend(
        (1..CAPACITY as u32).map(|i| cube_at(mesh, off_screen, 0.1, [0.0, 0.0, 1.0, 1.0], i)),
    );
    // Past the capacity: a green cube in front of the red one. If it were drawn, the centre
    // pixel would be green.
    objects.extend((0..EXTRA as u32).map(|i| {
        cube_at(
            mesh,
            Vec3::new(0.0, 0.0, -5.0),
            0.5,
            [0.0, 1.0, 0.0, 1.0],
            CAPACITY as u32 + i,
        )
    }));

    let frame = FrameView::new(&camera, &objects, &lights);
    renderer
        .render_frame_view(&frame, &view)
        .expect("a frame with too many objects must still render");
    let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
    let pixels = read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)
        .unwrap()
        .levels
        .swap_remove(0);

    let centre = ((SIZE / 2) * SIZE + SIZE / 2) as usize * 4;
    let (r, g) = (pixels[centre], pixels[centre + 1]);
    assert!(
        r > 2 * g,
        "centre pixel should be the red object drawn from the first {CAPACITY} objects, got r={r} g={g}"
    );
}
