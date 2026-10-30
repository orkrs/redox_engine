//! GPU checks of the texture nodes of material graphs (MAT-03 step 1): a graph texture samples
//! the bound texture like the legacy albedo texture, an empty texture list changes nothing, and
//! normal-map and triplanar nodes draw. Own binary: every test makes renderers, and the
//! process-global pipeline cache keeps each device alive. Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 96;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Each test makes renderers; running them at once can stall the driver, so they take turns.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    cube: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    let cube = renderer.upload_mesh(&create_cube());
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mat03_tex_target"),
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
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.4);
    Some(Rig {
        renderer,
        target,
        view,
        cube,
        camera,
        lights,
    })
}

impl Rig {
    fn frame(&mut self, objects: &[RenderObject]) -> Vec<u8> {
        let view = FrameView::new(&self.camera, objects, &self.lights);
        self.renderer
            .render_frame_view(&view, &self.view)
            .expect("frame");
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        read_texture_ktx2(
            &self.renderer.device,
            &self.renderer.queue,
            &self.target,
            false,
        )
        .unwrap()
        .levels
        .swap_remove(0)
    }

    /// Renders `n` identical frames and returns the last.
    fn settle(&mut self, objects: &[RenderObject], n: usize) -> Vec<u8> {
        let mut last = Vec::new();
        for _ in 0..n {
            last = self.frame(objects);
        }
        last
    }

    /// Unit cube in front of the camera, drawn with `material`.
    fn cube(&self, material: usize) -> RenderObject {
        let centre = Vec3::new(0.0, 0.0, -3.0);
        RenderObject {
            id: 1,
            shadow_class: ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(Vec3::ONE, Quat::IDENTITY, centre),
            color: [1.0; 4],
            mesh_index: self.cube,
            material_index: material,
            bounds: Aabb::from_center_size(centre, Vec3::ONE),
        }
    }
}

/// RGB at the middle of a frame.
fn centre(picture: &[u8]) -> [u8; 3] {
    let i = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    [picture[i], picture[i + 1], picture[i + 2]]
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// A 2x2 PNG filled with `colour`, as the renderer's texture upload reads it.
fn png(colour: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba(colour));
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

// ---------------------------------------------------------------- graphs

fn link(node: u32) -> Option<PortRef> {
    Some(PortRef { node, port: 0 })
}

fn node(id: u32, kind: NodeKind, inputs: Vec<Option<PortRef>>) -> Node {
    Node { id, kind, inputs }
}

fn graph(nodes: Vec<Node>) -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "mat03_tex".into(),
        nodes,
    }
}

/// Constant colour straight to the surface.
fn colour_graph(colour: [f32; 3]) -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::ConstVec3(colour), vec![]),
        node(
            2,
            NodeKind::SurfaceOutput,
            vec![link(1), None, None, None, None, None],
        ),
    ])
}

/// Slot 0 sampled at the uv and written to the base colour (rgb of the sample).
fn albedo_texture_graph() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::Uv, vec![]),
        node(2, NodeKind::Texture2d { slot: 0 }, vec![link(1)]),
        node(3, NodeKind::Split { component: 0 }, vec![link(2)]),
        node(4, NodeKind::Split { component: 1 }, vec![link(2)]),
        node(5, NodeKind::Split { component: 2 }, vec![link(2)]),
        node(6, NodeKind::Merge, vec![link(3), link(4), link(5)]),
        node(
            7,
            NodeKind::SurfaceOutput,
            vec![link(6), None, None, None, None, None],
        ),
    ])
}

/// Normal map of slot 0 on the normal port; triplanar sample of slot 0 on the base colour.
fn normal_triplanar_graph() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::Uv, vec![]),
        node(2, NodeKind::NormalMap { slot: 0 }, vec![link(1)]),
        node(3, NodeKind::Normal, vec![]),
        node(4, NodeKind::WorldPos, vec![]),
        node(
            5,
            NodeKind::Triplanar {
                slot: 0,
                scale: 2.0,
            },
            vec![link(4), link(3)],
        ),
        node(6, NodeKind::Split { component: 0 }, vec![link(5)]),
        node(7, NodeKind::Split { component: 1 }, vec![link(5)]),
        node(8, NodeKind::Split { component: 2 }, vec![link(5)]),
        node(9, NodeKind::Merge, vec![link(6), link(7), link(8)]),
        node(
            10,
            NodeKind::SurfaceOutput,
            vec![link(9), None, link(2), None, None, None],
        ),
    ])
}

// ---------------------------------------------------------------- tests

#[test]
fn a_graph_with_a_texture_matches_the_legacy_albedo_texture() {
    let _guard = lock();
    let Some(mut legacy) = rig() else {
        return;
    };
    let texture = legacy
        .renderer
        .upload_texture(&png([200, 60, 30, 255]), "mat03_albedo")
        .unwrap();
    let material = legacy
        .renderer
        .add_material(Material::textured(Vec3::ONE, texture));
    let object = legacy.cube(material);
    let legacy_picture = legacy.settle(&[object], 10);
    drop(legacy);

    let Some(mut graphed) = rig() else {
        return;
    };
    let texture = graphed
        .renderer
        .upload_texture(&png([200, 60, 30, 255]), "mat03_albedo")
        .unwrap();
    let material = graphed.renderer.add_material(Material::solid(Vec3::ONE));
    graphed
        .renderer
        .set_material_graph_with_textures(material, &albedo_texture_graph(), &[], &[texture])
        .expect("graph");
    let object = graphed.cube(material);
    let graph_picture = graphed.settle(&[object], 10);

    let (a, b) = (centre(&legacy_picture), centre(&graph_picture));
    assert!(
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 4),
        "legacy {a:?}, graph {b:?}"
    );
    // A white fallback would have no red excess: the texture itself is sampled.
    assert!(b[0] > b[1], "graph colour {b:?} is not the texture");
}

#[test]
fn a_graph_without_textures_is_bitwise_unchanged() {
    let _guard = lock();
    let graph = colour_graph([0.2, 0.6, 0.9]);
    // Two fresh renderers: the frame state of one renderer (temporal accumulation) would
    // otherwise differ between the two frames that are compared.
    let mut pictures = Vec::new();
    for with_textures_call in [false, true] {
        let Some(mut rig) = rig() else {
            return;
        };
        let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
        if with_textures_call {
            rig.renderer
                .set_material_graph_with_textures(material, &graph, &[], &[])
                .expect("graph");
        } else {
            rig.renderer
                .set_material_graph(material, &graph, &[])
                .expect("graph");
        }
        let object = rig.cube(material);
        pictures.push(rig.settle(&[object], 4));
    }

    assert_eq!(max_diff(&pictures[0], &pictures[1]), 0, "the frame changed");
}

#[test]
fn a_normal_map_and_triplanar_graph_draws() {
    let _guard = lock();
    let Some(mut rig) = rig() else {
        return;
    };
    let texture = rig
        .renderer
        .upload_texture(&png([128, 128, 255, 255]), "mat03_normal")
        .unwrap();
    let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph_with_textures(material, &normal_triplanar_graph(), &[], &[texture])
        .expect("graph");
    let object = rig.cube(material);
    let picture = rig.settle(&[object], 6);

    let background = [picture[0], picture[1], picture[2]];
    assert_ne!(centre(&picture), background, "the cube was not drawn");
}
