//! TZ §23.6: the editor finds the scenes that use an asset; deleting it without `force` is
//! refused; with `force` the references stay in the data and report as missing.

use crate::common::{CHECKER_GUID, Deletion, EditorHarness};

const CHECKER: &str = "assets/textures/checker.png";

#[test]
fn dependent_scenes_are_found_and_a_referenced_asset_is_not_deleted_without_force() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");

    assert_eq!(
        h.dependent_scenes(CHECKER),
        vec!["scenes/main.scene.ron".to_owned()],
        "the main scene uses the texture"
    );

    let outcome = h
        .delete_asset(CHECKER, false)
        .expect("the delete request is answered");
    assert_eq!(
        outcome,
        Deletion::Blocked(vec!["scenes/main.scene.ron".to_owned()]),
        "delete without force lists the dependent scene"
    );
    assert!(h.file_exists(CHECKER), "the asset is still there");
}

#[test]
fn a_forced_delete_leaves_missing_references_that_are_still_in_the_data() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");

    let outcome = h
        .delete_asset(CHECKER, true)
        .expect("the forced delete succeeds");
    assert_eq!(outcome, Deletion::Deleted);
    assert!(!h.file_exists(CHECKER), "the file is gone");

    let mut h = h.restart();
    h.open_project(&dir).expect("the project opens again");
    h.open_start_scene().expect("the main scene opens");

    assert!(
        h.load_warning_tags().iter().any(|t| t == "MissingAsset"),
        "the scene reports MissingAsset: {:?}",
        h.load_warning_tags()
    );
    let ground = h.find_by_name("Ground").expect("Ground is still there");
    let (guid, path) = h
        .asset_ref(ground, "MaterialHandle")
        .expect("the reference is not removed");
    assert_eq!(guid.as_deref(), Some(format!("{CHECKER_GUID:032x}").as_str()));
    assert_eq!(path.as_deref(), Some(CHECKER));
}
