//! GPU checks of the vertex offset (WPO) of graph materials in the main, depth and sun shadow
//! passes (MAT-03 step 4, MAT-07 step 1). Own binary: every test makes a renderer, and the
//! process-global pipeline cache keeps each device alive. Skipped without a GPU.

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
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Tests create two renderers each; running them at once can stall the driver, so they take turns.
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

fn rig(taa: bool) -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(taa);
    let cube = renderer.upload_mesh(&create_cube());
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wave7_target"),
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
    fn object(
        &self,
        mesh: usize,
        material: usize,
        id: u32,
        center: Vec3,
        scale: Vec3,
    ) -> RenderObject {
        RenderObject {
            id,
            shadow_class: ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
            color: [1.0; 4],
            mesh_index: mesh,
            material_index: material,
            bounds: Aabb::from_center_size(center, scale),
        }
    }

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
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------- vertex offset (WPO)

use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef};

fn link(node: u32) -> Option<PortRef> {
    Some(PortRef { node, port: 0 })
}

/// Red surface; `offset` adds a `PositionOffset` of that vector (declared maximum `max`).
fn wpo_graph(offset: Option<([f32; 3], f32)>) -> MaterialGraph {
    let mut nodes = vec![
        Node {
            id: 1,
            kind: NodeKind::ConstVec3([0.8, 0.2, 0.1]),
            inputs: vec![],
        },
        Node {
            id: 2,
            kind: NodeKind::SurfaceOutput,
            inputs: vec![link(1), None, None, None, None, None],
        },
    ];
    if let Some((v, max)) = offset {
        nodes.push(Node {
            id: 3,
            kind: NodeKind::ConstVec3(v),
            inputs: vec![],
        });
        nodes.push(Node {
            id: 4,
            kind: NodeKind::PositionOffset { max_offset: max },
            inputs: vec![link(3)],
        });
    }
    MaterialGraph {
        version: 1,
        name: "wpo_test".into(),
        nodes,
    }
}

/// A red graph cube at `centre` over a ground plane. `offset` is the graph's vertex offset.
fn wpo_frame(centre: Vec3, offset: Option<([f32; 3], f32)>) -> Option<Vec<u8>> {
    let mut rig = rig(false)?;
    if !rig.renderer.vsm_enabled {
        return None;
    }
    let ground_mat = rig.renderer.add_material(Material::solid(Vec3::splat(0.7)));
    let cube_mat = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph(cube_mat, &wpo_graph(offset), &[])
        .expect("graph");
    let ground = rig.object(
        rig.cube,
        ground_mat,
        1,
        Vec3::new(0.0, -1.0, -8.0),
        Vec3::new(40.0, 0.2, 40.0),
    );
    let cube = rig.object(rig.cube, cube_mat, 2, centre, Vec3::ONE);
    Some(rig.settle(&[ground, cube], 10))
}

/// A graph cube moved by the vertex offset looks like the same cube moved by its transform:
/// the main pass, the prepass and the sun's shadow pages all follow the displaced vertices.
#[test]
fn wpo_moves_the_vertices_and_the_shadows() {
    let _gpu = lock();
    let (Some(offset), Some(moved), Some(still)) = (
        wpo_frame(Vec3::new(-1.0, 0.5, -6.0), Some(([2.0, 0.0, 0.0], 2.0))),
        wpo_frame(Vec3::new(1.0, 0.5, -6.0), None),
        wpo_frame(Vec3::new(-1.0, 0.5, -6.0), None),
    ) else {
        return;
    };
    assert!(
        max_diff(&offset, &still) > 8,
        "the offset did not move the cube"
    );
    let diff = max_diff(&offset, &moved);
    assert!(
        diff <= 3,
        "an offset cube differs from the same cube moved by {diff}/255 (picture or shadow)"
    );
}

/// Without an offset node the graph pass renders exactly as an offset of zero.
#[test]
fn a_material_without_wpo_is_bitwise_unchanged() {
    let _gpu = lock();
    let (Some(plain), Some(zero)) = (
        wpo_frame(Vec3::new(0.0, 0.5, -6.0), None),
        wpo_frame(Vec3::new(0.0, 0.5, -6.0), Some(([0.0, 0.0, 0.0], 1.0))),
    ) else {
        return;
    };
    assert_eq!(max_diff(&plain, &zero), 0);
}

/// Culling uses the declared maximum offset: a cube whose own bounds are outside the view but
/// whose offset brings it in is still drawn.
#[test]
fn bounds_grow_by_the_declared_wpo_offset() {
    let _gpu = lock();
    let (Some(offset), Some(moved)) = (
        wpo_frame(Vec3::new(-9.0, 0.5, -6.0), Some(([9.0, 0.0, 0.0], 9.0))),
        wpo_frame(Vec3::new(0.0, 0.5, -6.0), None),
    ) else {
        return;
    };
    let diff = max_diff(&offset, &moved);
    assert!(
        diff <= 3,
        "the culled-looking cube vanished (diff {diff}/255)"
    );
}
