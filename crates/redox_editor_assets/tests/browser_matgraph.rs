//! `.matgraph` kind detection (EDT-B5 step 6).

use redox_editor_assets::{AssetAction, AssetKind, actions_for, BrowserEntry};
use redox_asset::AssetGuid;

#[test]
fn matgraph_kind_and_open_action() {
    assert_eq!(
        AssetKind::from_path(std::path::Path::new("materials/tint.matgraph")),
        AssetKind::Material
    );
    let entry = BrowserEntry {
        path: "materials/tint.matgraph".into(),
        name: "tint.matgraph".into(),
        guid: AssetGuid::default(),
        kind: AssetKind::Material,
        status: None,
    };
    let actions = actions_for(&entry);
    assert!(actions.contains(&AssetAction::Open));
}