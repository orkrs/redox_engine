//! Tests for GUID-level scene diff.

use redox_editor_scene::{
    SceneEntityChange, SceneFile, diff_scene_files,
};
use redox_editor_scene::format::SceneEntity;
use redox_reflect::ReflectValue;
use redox_scene::EntityGuid;

fn entity_with_name(guid: EntityGuid, name: &str) -> SceneEntity {
    SceneEntity {
        guid: Some(guid),
        components: vec![ReflectValue::Struct {
            type_name: "Name".into(),
            fields: vec![("name".into(), ReflectValue::String(name.into()))],
        }],
        unknown_components: Vec::new(),
    }
}

#[test]
fn add_entity_reports_added() {
    let guid = EntityGuid::new_random();
    let old = SceneFile {
        format_version: 1,
        name: "a".into(),
        entities: Vec::new(),
    };
    let new = SceneFile {
        format_version: 1,
        name: "a".into(),
        entities: vec![entity_with_name(guid, "cube")],
    };
    let changes = diff_scene_files(&old, &new);
    assert_eq!(
        changes,
        vec![SceneEntityChange::Added { guid }]
    );
}

#[test]
fn renamed_component_reports_one_changed() {
    let guid = EntityGuid::new_random();
    let old = SceneFile {
        format_version: 1,
        name: "a".into(),
        entities: vec![entity_with_name(guid, "crate")],
    };
    let new = SceneFile {
        format_version: 1,
        name: "a".into(),
        entities: vec![entity_with_name(guid, "crate_top")],
    };
    let changes = diff_scene_files(&old, &new);
    assert_eq!(changes.len(), 1);
    match &changes[0] {
        SceneEntityChange::Changed {
            guid: g,
            changed_components,
        } => {
            assert_eq!(*g, guid);
            assert_eq!(changed_components, &["Name".to_string()]);
        }
        other => panic!("expected Changed, got {other:?}"),
    }
}