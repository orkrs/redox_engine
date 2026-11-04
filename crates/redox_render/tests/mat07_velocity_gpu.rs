//! Velocity of vertices a graph material offsets (WPO, MAT-07 step 1): the velocity texture must
//! carry the motion of the displaced surface even when the model matrix stands still. Own binary:
//! every test makes a renderer. Skipped without a GPU.

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
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
/// Largest sub-pixel TAA jitter in UV plus rounding.
const JITTER: f32 = 0.02;

struct Rig {
    renderer: Renderer,
    target: wgpu::TextureView,
    mesh: usize,
    material: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(true);
    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
    let target = common::target(&renderer.device, SIZE);
    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 5.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    Some(Rig {
        renderer,
        target,
        mesh,
        material,
        camera,
        lights,
    })
}

impl Rig {
    fn object(&self, center: Vec3, scale: Vec3) -> RenderObject {
        RenderObject {
            id: 0,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
            color: [1.0; 4],
            mesh_index: self.mesh,
            material_index: self.material,
            bounds: Aabb::from_center_size(center, scale),
        }
    }

    fn draw(&mut self, objects: &[RenderObject], ids: Option<&[u64]>) {
        let mut view = FrameView::new(&self.camera, objects, &self.lights);
        if let Some(ids) = ids {
            view = view.with_object_ids(ids);
        }
        self.renderer
            .render_frame_view(&view, &self.target)
            .unwrap();
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f32::from(bits & 0x03ff);
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        e => sign * (1.0 + fraction / 1024.0) * 2f32.powi(e - 15),
    }
}

fn link(node: u32) -> Option<PortRef> {
    Some(PortRef { node, port: 0 })
}

/// Grey surface; the vertex offset is `(speed * time, 0, 0)` for `Some(speed)` (a cube drifting
/// `speed` metres per second), or the constant `(0.5, 0, 0)` for `None`.
fn drift_graph(speed: Option<f32>) -> MaterialGraph {
    let mut nodes = vec![
        Node {
            id: 1,
            kind: NodeKind::ConstVec3([0.6, 0.6, 0.6]),
            inputs: vec![],
        },
        Node {
            id: 2,
            kind: NodeKind::SurfaceOutput,
            inputs: vec![link(1), None, None, None, None, None],
        },
        Node {
            id: 3,
            kind: NodeKind::ConstF32(0.0),
            inputs: vec![],
        },
    ];
    match speed {
        Some(speed) => {
            nodes.push(Node {
                id: 4,
                kind: NodeKind::Time,
                inputs: vec![],
            });
            nodes.push(Node {
                id: 5,
                kind: NodeKind::ConstF32(speed),
                inputs: vec![],
            });
            nodes.push(Node {
                id: 6,
                kind: NodeKind::Mul,
                inputs: vec![link(4), link(5)],
            });
            nodes.push(Node {
                id: 7,
                kind: NodeKind::Merge,
                inputs: vec![link(6), link(3), link(3)],
            });
        }
        None => {
            nodes.push(Node {
                id: 7,
                kind: NodeKind::ConstVec3([0.5, 0.0, 0.0]),
                inputs: vec![],
            });
        }
    }
    nodes.push(Node {
        id: 8,
        kind: NodeKind::PositionOffset { max_offset: 4.0 },
        inputs: vec![link(7)],
    });
    MaterialGraph {
        version: 1,
        name: "drift".into(),
        nodes,
    }
}

impl Rig {
    /// Horizontal velocity of every pixel of the middle row of the last frame.
    fn velocity_row(&self) -> Vec<f32> {
        let texture = &self.renderer.taa_velocity_tex;
        let image =
            read_texture_ktx2(&self.renderer.device, &self.renderer.queue, texture, false).unwrap();
        let w = self.renderer.internal_width;
        let y = SIZE / 2;
        let bytes = &image.levels[0];
        (0..w)
            .map(|x| {
                let at = ((y * w + x) * 8) as usize;
                f16_to_f32(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
            })
            .collect()
    }
}

/// A static cube with a time-driven vertex offset: its pixels carry the offset's motion.
#[test]
fn wpo_velocity_matches_the_displacement() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // 30 m/s at 60 frames per second is 0.5 m per frame: 0.115 of the screen width.
    rig.renderer
        .set_material_graph(rig.material, &drift_graph(Some(30.0)), &[])
        .expect("graph");
    let cube = rig.object(Vec3::ZERO, Vec3::ONE);
    for _ in 0..4 {
        rig.draw(std::slice::from_ref(&cube), None);
    }
    let row = rig.velocity_row();
    let strongest = row
        .iter()
        .copied()
        .fold(0.0f32, |a, v| if v.abs() > a.abs() { v } else { a });
    assert!(
        (strongest - 0.115).abs() < 0.04,
        "the displaced surface moves 0.115 of the screen per frame, got {strongest}"
    );
}

/// A constant offset does not move anything between frames: no object motion.
#[test]
fn wpo_velocity_is_zero_for_a_static_offset() {
    let Some(mut rig) = rig() else { return };
    rig.renderer
        .set_material_graph(rig.material, &drift_graph(None), &[])
        .expect("graph");
    let cube = rig.object(Vec3::ZERO, Vec3::ONE);
    for _ in 0..4 {
        rig.draw(std::slice::from_ref(&cube), None);
    }
    let row = rig.velocity_row();
    let strongest = row.iter().fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(strongest < JITTER, "a static offset moved: {strongest}");
}

/// A graph without an offset behaves like a plain material: the moving cube carries its motion.
#[test]
fn a_graph_without_wpo_has_the_same_velocity_as_a_plain_material() {
    let (Some(mut plain), Some(mut graphed)) = (rig(), rig()) else {
        return;
    };
    let graph = {
        let mut g = drift_graph(None);
        g.nodes
            .retain(|n| !matches!(n.kind, NodeKind::PositionOffset { .. }));
        g
    };
    graphed
        .renderer
        .set_material_graph(graphed.material, &graph, &[])
        .expect("graph");
    for rig in [&mut plain, &mut graphed] {
        let before = rig.object(Vec3::new(-0.25, 0.0, 0.0), Vec3::ONE);
        rig.draw(&[before], None);
        let after = rig.object(Vec3::new(0.25, 0.0, 0.0), Vec3::ONE);
        rig.draw(&[after], None);
    }
    let (a, b) = (plain.velocity_row(), graphed.velocity_row());
    let worst = a
        .iter()
        .zip(&b)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(
        worst < JITTER,
        "graph velocity differs from plain by {worst}"
    );
}
