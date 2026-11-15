//! GPU preview of a known material graph (EDT-B5 step 4). Coordinator runs with a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_editor_assets::SimplePreviewLight;
use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef, Ty};
use redox_math::Aabb;
use redox_math::{Mat4, Quat, Vec3};
use redox_render::Material;
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 96;

fn colour_graph(colour: [f32; 3]) -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "preview_colour".into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::ConstVec3(colour),
                inputs: vec![],
            },
            Node {
                id: 2,
                kind: NodeKind::SurfaceOutput,
                inputs: vec![
                    Some(PortRef { node: 1, port: 0 }),
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            },
        ],
    }
}

fn tint_graph() -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "edt_b5_preview".into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::Param {
                    name: "tint".into(),
                    ty: Ty::Vec3,
                    default: [0.8, 0.4, 0.2, 0.0],
                },
                inputs: vec![],
            },
            Node {
                id: 2,
                kind: NodeKind::SurfaceOutput,
                inputs: vec![
                    Some(PortRef { node: 1, port: 0 }),
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            },
        ],
    }
}

#[test]
#[ignore = "gpu: coordinator"]
fn material_preview_graph_matches_tint_within_tolerance() {
    let adapter = match gpu_adapter() {
        Some(a) => a,
        None => return,
    };
    let mut renderer = match gpu_renderer(&adapter, SIZE) {
        Some(r) => r,
        None => return,
    };
    let graph = tint_graph();
    let material = renderer.add_material(Material::solid(Vec3::ONE));
    renderer
        .set_material_graph(material, &graph, &[])
        .expect("bind graph");
    let mesh = renderer.upload_mesh(&create_cube());
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("edt_b5_mat_preview"),
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
    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let light = SimplePreviewLight::studio_fallback();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(light.direction[0], light.direction[1], light.direction[2]),
        Vec3::new(light.colour[0], light.colour[1], light.colour[2]),
        2.5,
    ));
    lights.ambient = Vec3::new(light.ambient[0], light.ambient[1], light.ambient[2]);
    let centre = Vec3::new(0.0, 0.0, -3.0);
    let object = RenderObject {
        id: 1,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::from_scale_rotation_translation(Vec3::ONE, Quat::IDENTITY, centre),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(centre, Vec3::ONE),
    };
    let objects = [object];
    let frame = FrameView::new(&camera, &objects, &lights);
    renderer.render_frame_view(&frame, &view).expect("draw");
    let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
    let rgba = read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)
        .expect("readback")
        .levels
        .swap_remove(0);
    let (r, g, b) = (rgba[0], rgba[1], rgba[2]);
    assert!(
        (r as i32 - 204).abs() <= 8 && (g as i32 - 102).abs() <= 8 && (b as i32 - 51).abs() <= 8,
        "unexpected tint rgb=({r},{g},{b})"
    );
}

#[test]
#[ignore = "gpu: coordinator"]
fn the_preview_sphere_shows_the_graph_colour() {
    let Some(adapter) = gpu_adapter() else {
        return;
    };
    let Some(mut renderer) = gpu_renderer(&adapter, SIZE) else {
        return;
    };
    let material = renderer.add_material(Material::solid(Vec3::ONE));
    renderer
        .set_material_graph(material, &colour_graph([0.0, 1.0, 0.0]), &[])
        .unwrap();
    let mesh = renderer.upload_mesh(&create_sphere(1.0, 24));
    let [_, g, _, _] = render_preview_pixel(&mut renderer, mesh, material);
    assert!(g > 140);
}

#[test]
#[ignore = "gpu: coordinator"]
fn a_failed_compile_keeps_the_previous_preview() {
    let Some(adapter) = gpu_adapter() else {
        return;
    };
    let Some(mut renderer) = gpu_renderer(&adapter, SIZE) else {
        return;
    };
    let material = renderer.add_material(Material::solid(Vec3::ONE));
    renderer
        .set_material_graph(material, &colour_graph([0.0, 1.0, 0.0]), &[])
        .unwrap();
    let mesh = renderer.upload_mesh(&create_sphere(1.0, 24));
    let green = render_preview_pixel(&mut renderer, mesh, material);
    let broken = MaterialGraph {
        version: 1,
        name: "broken".into(),
        nodes: vec![Node {
            id: 1,
            kind: NodeKind::ConstVec3([1.0, 0.0, 0.0]),
            inputs: vec![],
        }],
    };
    assert!(renderer.set_material_graph(material, &broken, &[]).is_err());
    assert_eq!(render_preview_pixel(&mut renderer, mesh, material), green);
}

fn render_preview_pixel(renderer: &mut Renderer, mesh: usize, material: usize) -> [u8; 4] {
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mat_preview_readback"),
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
    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 3.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let light = SimplePreviewLight::studio_fallback();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(light.direction[0], light.direction[1], light.direction[2]),
        Vec3::new(light.colour[0], light.colour[1], light.colour[2]),
        2.2,
    ));
    lights.ambient = Vec3::new(light.ambient[0], light.ambient[1], light.ambient[2]);
    renderer.update_light_buffer(&lights.to_legacy_uniform());
    let object = RenderObject {
        id: 1,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::IDENTITY,
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::ONE),
    };
    renderer
        .render_frame_view(&FrameView::new(&camera, &[object], &lights), &view)
        .unwrap();
    let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
    let rgba = read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)
        .unwrap()
        .levels
        .swap_remove(0);
    let i = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn gpu_adapter() -> Option<wgpu::Adapter> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
}

fn gpu_renderer(adapter: &wgpu::Adapter, size: u32) -> Option<Renderer> {
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::PRIMITIVE_INDEX
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::TEXTURE_COMPRESSION_BC),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .ok()?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width: size,
        height: size,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    Some(Renderer::from_shared_device(
        device, queue, adapter, None, config,
    ))
}
