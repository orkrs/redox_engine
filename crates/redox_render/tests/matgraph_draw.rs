//! Material graphs drawn by the renderer (MAT-02 step 5). Skipped without a GPU adapter.

#![allow(clippy::unwrap_used)]

mod common;

use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef, Ty};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::matgraph_passes::GraphPass;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::systems::RenderObject;

const SIZE: u32 = 96;

struct Rig {
    renderer: Renderer,
    camera: Camera,
    lights: LightScene,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(false);
    let camera = Camera::new(0.9, 1.0, 0.1, 100.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 9.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    Some(Rig {
        renderer,
        camera,
        lights,
    })
}

fn cube_at(mesh: usize, material: usize, x: f32) -> RenderObject {
    let center = Vec3::new(x, 0.0, 0.0);
    RenderObject {
        id: 0,
        shadow_class: redox_render::shadow::ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::from_scale_rotation_translation(
            Vec3::splat(0.8),
            Quat::from_rotation_y(0.4),
            center,
        ),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(center, Vec3::splat(0.8)),
    }
}

impl Rig {
    /// Renders `objects` and returns the RGBA8 picture.
    fn draw(&mut self, objects: &[RenderObject]) -> Vec<u8> {
        let target = self
            .renderer
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("matgraph_target"),
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
        let frame = FrameView::new(&self.camera, objects, &self.lights);
        self.renderer.render_frame_view(&frame, &view).unwrap();
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        read_texture_ktx2(&self.renderer.device, &self.renderer.queue, &target, false)
            .unwrap()
            .levels
            .swap_remove(0)
    }
}

/// RGB at pixel (`x`, `y`) of an RGBA8 picture of width `SIZE`.
fn pixel(picture: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [picture[i], picture[i + 1], picture[i + 2]]
}

/// Surface output of a constant colour.
fn colour_graph(colour: [f32; 3]) -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "colour".into(),
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

/// `base_color` is a parameter `tint` (vec3), default white.
fn tint_graph() -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "tint".into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::Param {
                    name: "tint".into(),
                    ty: Ty::Vec3,
                    default: [1.0, 1.0, 1.0, 0.0],
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

/// Has no output node, so checking rejects it.
fn broken_graph() -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "broken".into(),
        nodes: vec![Node {
            id: 1,
            kind: NodeKind::ConstVec3([1.0, 0.0, 0.0]),
            inputs: vec![],
        }],
    }
}

#[test]
fn a_graph_material_renders_the_expected_colour() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mesh = rig.renderer.upload_mesh(&create_cube());
    let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph(material, &colour_graph([1.0, 0.0, 0.0]), &[])
        .unwrap();
    let picture = rig.draw(&[cube_at(mesh, material, 0.0)]);
    // Centre of the cube: lit red, so red dominates clearly.
    let [r, g, b] = pixel(&picture, SIZE / 2, SIZE / 2);
    assert!(
        r > g.saturating_add(60) && r > b.saturating_add(60),
        "centre pixel is not red: {r} {g} {b}"
    );
}

#[test]
fn two_instances_share_one_shader_with_different_params() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mesh = rig.renderer.upload_mesh(&create_cube());
    let red_slot = rig.renderer.add_material(Material::solid(Vec3::ONE));
    let blue_slot = rig.renderer.add_material(Material::solid(Vec3::ONE));
    let graph = tint_graph();
    let red = [[1.0, 0.0, 0.0, 0.0]];
    let blue = [[0.0, 0.0, 1.0, 0.0]];
    rig.renderer
        .set_material_graph(red_slot, &graph, &red)
        .unwrap();
    rig.renderer
        .set_material_graph(blue_slot, &graph, &blue)
        .unwrap();

    let matgraphs = &rig.renderer.matgraphs;
    assert_eq!(
        matgraphs.shader_count(),
        3,
        "one module per codegen pass (main, depth, shadow) for both slots"
    );
    assert_eq!(
        matgraphs.pipeline_count(),
        5,
        "forward, alpha, additive, depth and shadow pipelines"
    );
    assert_eq!(matgraphs.material_count(), 2);

    // Cubes left and right of the centre: about pixel 31 and 65 at this camera.
    let picture = rig.draw(&[cube_at(mesh, red_slot, -1.5), cube_at(mesh, blue_slot, 1.5)]);
    let [lr, lg, lb] = pixel(&picture, 31, SIZE / 2);
    let [rr, rg, rb] = pixel(&picture, 65, SIZE / 2);
    assert!(
        lr > lg.saturating_add(60) && lr > lb.saturating_add(60),
        "left: {lr} {lg} {lb}"
    );
    assert!(
        rb > rr.saturating_add(60) && rb > rg.saturating_add(60),
        "right: {rr} {rg} {rb}"
    );
}

#[test]
fn a_broken_graph_keeps_the_previous_shader() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph(material, &colour_graph([0.0, 1.0, 0.0]), &[])
        .unwrap();
    let shaders_before = rig.renderer.matgraphs.shader_count();

    let result = rig
        .renderer
        .set_material_graph(material, &broken_graph(), &[]);
    assert!(result.is_err(), "a graph without output must be refused");
    assert!(
        rig.renderer.matgraphs.binding(material).is_some(),
        "the slot keeps its previous graph"
    );
    assert_eq!(rig.renderer.matgraphs.shader_count(), shaders_before);
}

#[test]
fn a_slot_without_a_graph_has_no_binding() {
    let Some(rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    assert!(rig.renderer.matgraphs.binding(0).is_none());
}

#[test]
fn the_debug_copy_of_a_variant_is_written_once() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("matgraph_disk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    rig.renderer.matgraphs.set_disk_cache(Some(dir.clone()));
    let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
    let key = rig
        .renderer
        .matgraphs
        .set_material(
            &rig.renderer.device,
            &rig.renderer.queue,
            material,
            &colour_graph([0.0, 0.0, 1.0]),
            &[],
        )
        .unwrap();
    let file = dir.join(format!("{:016x}.wgsl", key.0));
    let text = std::fs::read_to_string(&file).expect("debug copy is written");
    assert!(text.contains("material_surface_graph_variant"));
}

#[test]
fn a_graph_slot_has_a_pipeline_for_every_pass() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let graphed = rig.renderer.add_material(Material::solid(Vec3::ONE));
    let plain = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph(graphed, &colour_graph([0.0, 1.0, 0.0]), &[])
        .unwrap();
    for pass in GraphPass::ALL {
        assert!(
            rig.renderer.matgraphs.pass_binding(graphed, pass).is_some(),
            "graph slot has no {pass:?} pipeline"
        );
        assert!(
            rig.renderer.matgraphs.pass_binding(plain, pass).is_none(),
            "plain slot must keep the default {pass:?} pipeline"
        );
    }
}

#[test]
fn a_disk_cache_file_is_reused_only_while_its_header_matches() {
    use redox_render::matgraph_cache::{cache_text, graph_source_hash};

    let Some(mut first) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("matgraph_reuse_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let graph = colour_graph([0.0, 0.5, 1.0]);
    let hash = graph_source_hash(&graph);

    first.renderer.matgraphs.set_disk_cache(Some(dir.clone()));
    let material = first.renderer.add_material(Material::solid(Vec3::ONE));
    let key = first
        .renderer
        .matgraphs
        .set_material(
            &first.renderer.device,
            &first.renderer.queue,
            material,
            &graph,
            &[],
        )
        .unwrap();
    let file = dir.join(format!("{:016x}.wgsl", key.0));
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        written.starts_with("// matgraph-cache v1 "),
        "header written"
    );
    let body = written.split_once('\n').unwrap().1;

    // Matching header and body: a second renderer reads the file and does not rewrite it.
    let reused = cache_text(hash, &format!("{body}// read back from cache\n"));
    std::fs::write(&file, &reused).unwrap();
    let Some(mut second) = rig() else {
        return;
    };
    second.renderer.matgraphs.set_disk_cache(Some(dir.clone()));
    let material = second.renderer.add_material(Material::solid(Vec3::ONE));
    second
        .renderer
        .matgraphs
        .set_material(
            &second.renderer.device,
            &second.renderer.queue,
            material,
            &graph,
            &[],
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        reused,
        "a matching cache file is read, not rewritten"
    );

    // A file of another cache version is refused: the WGSL is generated again and the file is
    // rewritten with the current header.
    let stale =
        format!("// matgraph-cache v0 format=1 source={hash:016x} body=0000000000000000\n{body}");
    std::fs::write(&file, &stale).unwrap();
    let Some(mut third) = rig() else {
        return;
    };
    third.renderer.matgraphs.set_disk_cache(Some(dir.clone()));
    let material = third.renderer.add_material(Material::solid(Vec3::ONE));
    third
        .renderer
        .matgraphs
        .set_material(
            &third.renderer.device,
            &third.renderer.queue,
            material,
            &graph,
            &[],
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        written,
        "a stale cache file is regenerated"
    );
}

/// A `.matgraph` path unique to this test process and `name`.
fn temp_graph_file(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{name}_{}.matgraph", std::process::id()))
}

#[test]
fn a_changed_graph_file_rebuilds_the_slot_without_a_restart() {
    use redox_render::matgraph_cache::MaterialGraphHotReload;
    use std::time::Duration;

    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let path = temp_graph_file("matgraph_reload_ok");
    std::fs::write(&path, colour_graph([1.0, 0.0, 0.0]).to_ron()).unwrap();
    let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph(material, &colour_graph([1.0, 0.0, 0.0]), &[])
        .unwrap();
    let mut reload = MaterialGraphHotReload::with_interval(Duration::ZERO);
    reload.watch(material, &path, Vec::new());
    assert!(
        reload.update(&mut rig.renderer).is_empty(),
        "nothing changed yet"
    );
    let shaders_before = rig.renderer.matgraphs.shader_count();

    // Other colour text: a different length, so the watcher sees the change.
    std::fs::write(&path, colour_graph([0.123_456, 0.0, 0.0]).to_ron()).unwrap();
    let failures = reload.update(&mut rig.renderer);
    let _ = std::fs::remove_file(&path);
    assert!(failures.is_empty(), "{failures:?}");
    assert!(
        rig.renderer.matgraphs.shader_count() > shaders_before,
        "the changed graph built new variants"
    );
    assert!(
        rig.renderer
            .matgraphs
            .pass_binding(material, GraphPass::Forward)
            .is_some()
    );
}

#[test]
fn a_broken_graph_file_is_reported_once_and_the_old_graph_stays() {
    use redox_render::matgraph_cache::{MaterialGraphError, MaterialGraphHotReload};
    use std::time::Duration;

    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let path = temp_graph_file("matgraph_reload_broken");
    std::fs::write(&path, colour_graph([0.0, 1.0, 0.0]).to_ron()).unwrap();
    let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
    rig.renderer
        .set_material_graph(material, &colour_graph([0.0, 1.0, 0.0]), &[])
        .unwrap();
    let mut reload = MaterialGraphHotReload::with_interval(Duration::ZERO);
    reload.watch(material, &path, Vec::new());
    let shaders_before = rig.renderer.matgraphs.shader_count();

    std::fs::write(&path, "(this is not a graph)").unwrap();
    let failures = reload.update(&mut rig.renderer);
    assert_eq!(failures.len(), 1, "one failure for the broken file");
    assert_eq!(failures[0].material_index, material);
    assert!(matches!(failures[0].error, MaterialGraphError::Graph(_)));
    assert!(
        reload.update(&mut rig.renderer).is_empty(),
        "the same broken state is not reported again"
    );
    let _ = std::fs::remove_file(&path);
    assert_eq!(rig.renderer.matgraphs.shader_count(), shaders_before);
    assert!(
        rig.renderer
            .matgraphs
            .pass_binding(material, GraphPass::Forward)
            .is_some(),
        "the slot keeps its previous graph"
    );
}

#[test]
fn an_unchanged_graph_file_does_nothing() {
    use redox_render::matgraph_cache::MaterialGraphHotReload;
    use std::time::Duration;

    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let path = temp_graph_file("matgraph_reload_same");
    std::fs::write(&path, colour_graph([0.0, 0.0, 1.0]).to_ron()).unwrap();
    let material = rig.renderer.add_material(Material::solid(Vec3::ONE));
    let mut reload = MaterialGraphHotReload::with_interval(Duration::ZERO);
    reload.watch(material, &path, Vec::new());
    let shaders_before = rig.renderer.matgraphs.shader_count();
    assert!(reload.update(&mut rig.renderer).is_empty());
    assert!(reload.update(&mut rig.renderer).is_empty());
    let _ = std::fs::remove_file(&path);
    assert_eq!(rig.renderer.matgraphs.shader_count(), shaders_before);
    assert_eq!(
        rig.renderer.matgraphs.material_count(),
        0,
        "no graph was applied"
    );
}
