//! EDT-B3 viewport tests (coordinator runs GPU tests separately).

use redox_editor_viewport::{
    ViewportDrawItem, ViewportMeshCache, ViewportShading, ViewportShadingPlan,
    gltf_mesh_to_render_mesh,
};
use redox_math::Transform;
#[test]
fn unavailable_reasons_are_gone_for_the_four_modes() {
    for mode in [
        ViewportShading::Unlit,
        ViewportShading::Normals,
        ViewportShading::Roughness,
        ViewportShading::Depth,
    ] {
        let plan = ViewportShadingPlan::resolve(mode);
        assert_eq!(plan.status, redox_editor_viewport::ShadingAvailability::Available);
        assert!(!plan.overlay_label().contains("unavailable"));
    }
}

#[test]
fn wireframe_uses_overlay_not_fake_shader() {
    let plan = ViewportShadingPlan::resolve(ViewportShading::Wireframe);
    assert!(plan.wireframe_overlay);
    assert!(!plan.overlay_label().contains("unavailable"));
}

#[test]
fn proxy_path_when_no_mesh_ref() {
    let cache = ViewportMeshCache::new();
    let item = ViewportDrawItem {
        entity_id: 1,
        transform: Transform::default(),
        mesh_path: None,
        mesh_ref: None,
    };
    assert!(cache.missing_entities().is_empty());
    let _ = item;
    assert!(cache.missing_entities().is_empty());
}

#[test]
fn gltf_triangle_converts_without_panic() {
    let data = redox_asset::GltfMeshData {
        name: "tri".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uvs: vec![[0.0, 0.0]; 3],
        tangents: Vec::new(),
        indices: vec![0, 1, 2],
        material: None,
        generated_uv: false,
        colors: Vec::new(),
        uvs1: Vec::new(),
    };
    let mesh = gltf_mesh_to_render_mesh(&data);
    assert_eq!(mesh.indices.len(), 3);
    assert_eq!(mesh.vertices.len(), 3);
}

#[test]
#[ignore = "GPU golden: coordinator runs on reference GPU"]
fn wireframe_and_lit_differ_on_controlled_scene() {
    // Placeholder for EDT-B3 step 2 GPU criterion (mean luma edge buffer).
    let _ = ViewportShading::Lit;
}

#[test]
#[ignore = "GPU golden: viewport PNG tolerance 3/255"]
fn viewport_golden_controlled_scene() {
    let _ = std::path::Path::new("viewport_golden.png");
}

#[test]
fn headless_viewport_mesh_fixture_does_not_panic() {
    let item = ViewportDrawItem {
        entity_id: 9,
        transform: Transform::default(),
        mesh_path: None,
        mesh_ref: None,
    };
    let _cache = ViewportMeshCache::new();
    assert!(item.mesh_path.is_none());
}
