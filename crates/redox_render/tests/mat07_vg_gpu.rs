//! GPU checks of graph materials in the visibility buffer resolve of virtual geometry (MAT-07
//! step 2). Own binary: every test makes renderers, and the process-global pipeline cache keeps
//! each device alive. Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef};
use redox_math::{Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_sphere;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::virtual_geometry::{DagParams, VGConfig, build_vg_dag_asset};

const SIZE: u32 = 128;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Tests create renderers; running them at once can stall the driver, so they take turns.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    camera: Camera,
    lights: LightScene,
}

/// A renderer with virtual geometry on (DAG traversal, as `vg_dag_frame` draws it) and a square
/// target. The camera sits on +Z looking at the origin, so a unit sphere at the origin covers the
/// centre of the picture.
fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.init_virtual_geometry(VGConfig {
        dag_traverse: true,
        ..VGConfig::default()
    });
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mat07_vg_target"),
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
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, 1.0),
        Vec3::ONE,
        3.0,
    ));
    // The light direction is the one the shaders use as `l`: towards the light, so +Z lights the
    // side that faces the camera.
    lights.ambient = Vec3::splat(0.4);
    Some(Rig {
        renderer,
        target,
        view,
        camera,
        lights,
    })
}

impl Rig {
    /// Registers a sphere of `radius` with `material` as a DAG VG asset and spawns it at `at`.
    fn add_sphere(&mut self, material: usize, at: Vec3, radius: f32) {
        let mesh = create_sphere(radius, 32);
        let asset =
            build_vg_dag_asset(&mesh, &DagParams::default(), material as u32).expect("dag asset");
        let transform = Mat4::from_translation(at).to_cols_array_2d();
        let queue = &self.renderer.queue;
        let vg = self.renderer.vg_system.as_mut().expect("vg");
        let id = vg.register_dag_asset(asset, queue).expect("register");
        vg.spawn_dag_instance(id, transform).expect("spawn");
    }

    /// Renders one frame of the VG scene and reads the target back.
    fn frame(&mut self) -> Vec<u8> {
        self.renderer
            .camera_uniform
            .update(&self.camera, Vec3::new(0.0, 0.0, 6.0), Quat::IDENTITY);
        self.renderer.update_camera_buffer();
        let view = FrameView::new(&self.camera, &[], &self.lights);
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
    fn settle(&mut self, n: usize) -> Vec<u8> {
        let mut last = Vec::new();
        for _ in 0..n {
            last = self.frame();
        }
        last
    }
}

/// RGB of the pixel at (`x`, `y`) of a `SIZE` wide RGBA8 picture.
fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [bytes[i], bytes[i + 1], bytes[i + 2]]
}

fn centre(bytes: &[u8]) -> [u8; 3] {
    pixel(bytes, SIZE / 2, SIZE / 2)
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// A graph whose surface is the constant colour `rgb`.
fn const_graph(name: &str, rgb: [f32; 3]) -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: name.into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::ConstVec3(rgb),
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

/// A graph material on a VG mesh takes the colour of its graph; without one the base material
/// shows.
#[test]
fn a_graph_material_on_a_vg_mesh_changes_the_colour() {
    let _gpu = lock();
    let (Some(mut plain), Some(mut graphed)) = (rig(), rig()) else {
        return;
    };
    let base = plain
        .renderer
        .add_material(Material::solid(Vec3::new(0.9, 0.1, 0.1)));
    plain.add_sphere(base, Vec3::ZERO, 1.0);
    let before = centre(&plain.settle(8));
    assert!(
        before[0] > before[1] + 40,
        "base red is not red: {before:?}"
    );

    let slot = graphed
        .renderer
        .add_material(Material::solid(Vec3::new(0.9, 0.1, 0.1)));
    graphed.add_sphere(slot, Vec3::ZERO, 1.0);
    graphed
        .renderer
        .set_material_graph(slot, &const_graph("vg_green", [0.1, 0.9, 0.1]), &[])
        .expect("graph");
    let after = centre(&graphed.settle(8));
    assert!(
        after[1] > after[0] + 40,
        "the graph colour does not show on the VG mesh: {after:?}"
    );
}

/// Two graph materials in one VG frame: each sphere shows its own graph.
#[test]
fn two_graph_materials_in_one_vg_frame() {
    let _gpu = lock();
    let Some(mut rig) = rig() else { return };
    let red = rig.renderer.add_material(Material::solid(Vec3::ONE));
    let blue = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph(red, &const_graph("vg_red", [0.9, 0.1, 0.1]), &[])
        .expect("red graph");
    rig.renderer
        .set_material_graph(blue, &const_graph("vg_blue", [0.1, 0.1, 0.9]), &[])
        .expect("blue graph");
    rig.add_sphere(red, Vec3::new(-1.2, 0.0, 0.0), 1.0);
    rig.add_sphere(blue, Vec3::new(1.2, 0.0, 0.0), 1.0);
    let picture = rig.settle(8);
    let left = pixel(&picture, 38, SIZE / 2);
    let right = pixel(&picture, 90, SIZE / 2);
    assert!(left[0] > left[2] + 40, "left sphere is not red: {left:?}");
    assert!(
        right[2] > right[0] + 40,
        "right sphere is not blue: {right:?}"
    );
}

/// More graph slots than the resolve draws: the frame still renders and the spheres keep a
/// shading instead of crashing.
#[test]
fn more_than_eight_graph_materials_fall_back_without_crashing() {
    let _gpu = lock();
    let Some(mut rig) = rig() else { return };
    let mut slots = Vec::new();
    for i in 0..10 {
        let slot = rig.renderer.add_material(Material::solid(Vec3::ONE));
        let shade = i as f32 / 10.0;
        rig.renderer
            .set_material_graph(slot, &const_graph("vg_many", [shade, 0.2, 0.5]), &[])
            .expect("graph");
        slots.push(slot);
    }
    rig.add_sphere(slots[9], Vec3::ZERO, 1.0);
    let picture = rig.settle(4);
    assert!(
        centre(&picture).iter().any(|&c| c > 0),
        "the sphere did not render at all"
    );
}

/// A graph on a slot no VG mesh uses changes no pixel of a VG scene.
#[test]
fn vg_without_graph_materials_is_bitwise_unchanged() {
    let _gpu = lock();
    let (Some(mut plain), Some(mut with_unused)) = (rig(), rig()) else {
        return;
    };
    let base = plain
        .renderer
        .add_material(Material::solid(Vec3::new(0.9, 0.1, 0.1)));
    plain.add_sphere(base, Vec3::ZERO, 1.0);
    let base_b = with_unused
        .renderer
        .add_material(Material::solid(Vec3::new(0.9, 0.1, 0.1)));
    let unused = with_unused
        .renderer
        .add_material(Material::solid(Vec3::ONE));
    with_unused
        .renderer
        .set_material_graph(unused, &const_graph("vg_unused", [0.1, 0.9, 0.1]), &[])
        .expect("graph");
    with_unused.add_sphere(base_b, Vec3::ZERO, 1.0);
    let a = plain.settle(8);
    let b = with_unused.settle(8);
    assert_eq!(max_diff(&a, &b), 0);
}
