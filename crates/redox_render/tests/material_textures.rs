//! A material that refers to a KTX2 texture really samples it when a frame is drawn.

mod common;

use redox_asset::{AssetManager, Handle, Ktx2Texture, TextureFormat};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::asset_types::{MaterialData, MeshData};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::systems::{RenderObject, sync_ktx2_textures_to_render};

const SIZE: u32 = 96;

fn solid(color: [u8; 4]) -> Ktx2Texture {
    let mut levels = Vec::new();
    let mut s = 8u32;
    loop {
        levels.push(color.repeat((s * s) as usize));
        if s == 1 {
            break;
        }
        s /= 2;
    }
    Ktx2Texture::new_2d(8, 8, TextureFormat::Rgba8Unorm, levels)
}

/// Renders a lit cube textured with `ktx` and returns the color at the middle of the frame.
fn center_pixel(ktx: Ktx2Texture) -> Option<[u8; 3]> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("material_test_target"),
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

    let mut assets = AssetManager::new(".");
    let texture: Handle<Ktx2Texture> = assets.insert(ktx);
    sync_ktx2_textures_to_render(&mut renderer, &assets, &[texture]);
    let material = assets.insert(MaterialData::solid(Vec3::ONE).with_albedo_ktx2(texture));
    let mesh: Handle<MeshData> = assets.insert(create_cube());
    let mesh_index = renderer
        .add_mesh_from_asset(mesh, assets.get(mesh).unwrap())
        .unwrap();
    let material_index = renderer
        .add_material_from_asset(material, assets.get(material).unwrap())
        .unwrap();

    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    let position = Vec3::new(0.0, 0.0, 2.2);
    renderer
        .camera_uniform
        .update(&camera, position, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, 0.3, 1.0),
        Vec3::ONE,
        2.0,
    ));
    lights.ambient = Vec3::splat(0.8);
    let objects = [RenderObject {
        model_matrix: Mat4::IDENTITY,
        color: [1.0; 4],
        mesh_index,
        material_index,
        bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5)),
    }];
    for _ in 0..4 {
        let fv = FrameView::new(&camera, &objects, &lights);
        renderer.render_frame_view(&fv, &view).expect("frame");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
    }
    let image = read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)?;
    let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    let pixel = &image.levels[0][at..at + 3];
    Some([pixel[0], pixel[1], pixel[2]])
}

#[test]
fn ktx2_textures_color_the_object_that_uses_them() {
    let Some(red) = center_pixel(solid([255, 0, 0, 255])) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let green = center_pixel(solid([0, 255, 0, 255])).unwrap();
    eprintln!("center pixel: red texture -> {red:?}, green texture -> {green:?}");
    assert!(
        red[0] > red[1].saturating_add(40) && red[0] > red[2].saturating_add(40),
        "red texture should give a red object: {red:?}"
    );
    assert!(
        green[1] > green[0].saturating_add(40) && green[1] > green[2].saturating_add(40),
        "green texture should give a green object: {green:?}"
    );
}
