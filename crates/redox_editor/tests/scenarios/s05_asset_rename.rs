//! TZ §23.5: renaming and moving an asset keeps every reference intact in an open scene and in
//! a closed scene, also after a restart.

use crate::common::{CHECKER_GUID, EditorHarness, SceneEntitySpec, make_scene, AssetRef};

const RENAMED: &str = "assets/textures/checker_renamed.png";
const MOVED: &str = "assets/textures/nested/checker_moved.png";
const CHECKER: &str = "assets/textures/checker.png";

/// A second scene that references the checker texture and stays closed.
fn write_side_scene(h: &EditorHarness) {
    let text = make_scene(
        "Side",
        &[SceneEntitySpec {
            guid: 0x10,
            name: "Side Ground".into(),
            translation: [0.0, 0.0, 0.0],
            parent: None,
            asset: Some(AssetRef {
                type_name: "MaterialHandle".into(),
                guid: CHECKER_GUID,
                path: CHECKER.into(),
            }),
        }],
    );
    h.write_file("scenes/side.scene.ron", text.as_bytes());
}

#[test]
fn renaming_and_moving_an_asset_keeps_references_in_open_and_closed_scenes() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");
    write_side_scene(&h);
    h.open_start_scene().expect("the main scene opens");

    h.move_asset(CHECKER, RENAMED, true)
        .expect("the asset is renamed");
    h.move_asset(RENAMED, MOVED, true)
        .expect("the asset is moved");
    h.save().expect("the open scene is saved");

    let mut h = h.restart();
    h.open_project(&dir).expect("the project opens again");
    h.open_start_scene().expect("the main scene opens again");
    let ground = h.find_by_name("Ground").expect("Ground is in the main scene");
    let (guid, path) = h
        .asset_ref(ground, "MaterialHandle")
        .expect("Ground still references the texture");
    assert_eq!(guid.as_deref(), Some(format!("{CHECKER_GUID:032x}").as_str()));
    assert_eq!(path.as_deref(), Some(MOVED), "the hint follows the move");
    assert!(h.load_warning_tags().is_empty(), "no MissingAsset after the move");

    h.open_scene("scenes/side.scene.ron")
        .expect("the closed scene opens");
    let side = h.find_by_name("Side Ground").expect("Side Ground exists");
    let (_, path) = h
        .asset_ref(side, "MaterialHandle")
        .expect("Side Ground still references the texture");
    assert_eq!(path.as_deref(), Some(MOVED), "the closed scene was rewritten");
    assert!(h.load_warning_tags().is_empty());
}
