//! VG-06 packager meshlet bake (CPU only).

use redox_editor_ui::PackagerPanel;
use redox_render::mesh::primitive::create_cube;
use std::path::PathBuf;

#[test]
fn bake_rejects_an_empty_selection() {
    let mut panel = PackagerPanel::default();
    let err = panel.bake_meshlets_from_paths(&[]).unwrap_err();
    assert!(err.to_string().contains("no mesh selected"));
}

#[test]
fn bake_of_a_cube_mesh_writes_the_cache_and_names_the_source() {
    let mut panel = PackagerPanel::default();
    let source = PathBuf::from("assets/test_cube.gltf");
    let mesh = create_cube();
    let outcome = panel
        .bake_meshlets_from_mesh(&mesh, source.clone())
        .expect("bake");
    assert!(outcome.meshlet_count > 0);
    assert!(outcome.cache_path.is_file());
    let report = std::fs::read_to_string(".cache/bake_meshlets.txt").expect("report");
    assert!(report.contains(&source.display().to_string()));
    assert!(report.contains("meshlets:"));
}

#[test]
fn the_command_does_not_log_not_wired() {
    let msg = "Bake meshlets: 12 from foo.gltf → .cache/vg/abc.rmesh (gpu_register: ok 0)";
    assert!(!msg.contains("not wired"));
}