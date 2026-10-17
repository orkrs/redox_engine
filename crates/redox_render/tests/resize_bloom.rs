//! After a window resize the bloom of the old size must not stay on screen.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::systems::RenderObject;

fn target(r: &Renderer, size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resize_bloom_target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn draw(r: &mut Renderer, view: &wgpu::TextureView, objects: &[RenderObject]) {
    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    r.camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 2.5), Quat::IDENTITY);
    r.update_camera_buffer();
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        0.5,
    ));
    for _ in 0..16 {
        let fv = FrameView::new(&camera, objects, &lights);
        r.render_frame_view(&fv, view).expect("frame");
        let _ = r.device.poll(wgpu::PollType::wait_indefinitely());
    }
}

fn brightness(r: &Renderer, texture: &wgpu::Texture, x: u32, y: u32, size: u32) -> f32 {
    let image = read_texture_ktx2(&r.device, &r.queue, texture, false).unwrap();
    let at = ((y * size + x) * 4) as usize;
    image.levels[0][at..at + 3]
        .iter()
        .map(|c| *c as f32)
        .sum::<f32>()
        / 3.0
}

/// How the frame is anti-aliased and tone mapped; each mode binds the bloom texture differently.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Plain,
    Taa,
    Sharp,
}

fn check(mode: Mode) {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, 96) else {
        return;
    };
    match mode {
        Mode::Plain => {
            renderer.enable_taa(false);
            renderer.set_aa_sharp(false);
        }
        Mode::Taa => renderer.enable_taa(true),
        Mode::Sharp => renderer.set_aa_sharp(true),
    }
    renderer.bloom_enabled = true;
    renderer.set_exposure(0.15);

    // A very bright emissive cube makes a strong bloom.
    let mut glow = Material::solid(Vec3::ONE);
    glow.emissive = Vec3::splat(40.0);
    let material = renderer.add_material(glow);
    let mesh = renderer.upload_mesh(&create_cube());
    let cube = RenderObject {
        model_matrix: Mat4::IDENTITY,
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5)),
    };
    let (_t, view) = target(&renderer, 96);
    draw(&mut renderer, &view, &[cube]);

    renderer.resize(128, 128);
    let (texture, view) = target(&renderer, 128);
    draw(&mut renderer, &view, &[]);

    // The scene is empty now, so the whole frame is plain sky.
    let centre = brightness(&renderer, &texture, 64, 64, 128);
    let corner = brightness(&renderer, &texture, 6, 6, 128);
    // Plain sky is about 20 here; a leftover bloom pushes it towards white.
    assert!(
        centre < 60.0 && corner < 60.0,
        "{mode:?}: the old bloom is still on screen: centre {centre}, corner {corner}"
    );
}

#[test]
fn bloom_from_before_a_resize_does_not_linger_without_aa() {
    check(Mode::Plain);
}

#[test]
fn bloom_from_before_a_resize_does_not_linger_with_taa() {
    check(Mode::Taa);
}

#[test]
fn bloom_from_before_a_resize_does_not_linger_with_sharp_aa() {
    check(Mode::Sharp);
}
