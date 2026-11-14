//! [`AssetRefs`] reflection and RON roundtrip.

use redox_ecs::World;
use redox_reflect::{Reflect, ReflectValue};
use redox_scene::{AssetKind, AssetRef, AssetRefs};

#[test]
fn asset_refs_value_roundtrips_through_reflect() {
    let mut refs = AssetRefs::default();
    refs.set(
        "MaterialHandle",
        Some("00000000000000000000000000001001".into()),
        Some("assets/textures/checker.png".into()),
    );
    let value = refs.to_value();
    let mut restored = AssetRefs::default();
    restored.apply(&value).expect("apply");
    assert_eq!(restored, refs);
    assert_eq!(restored.to_value(), value);
}

#[test]
fn asset_refs_survive_ron() {
    let mut refs = AssetRefs::default();
    refs.set(
        "MeshHandle",
        Some("00000000000000000000000000001002".into()),
        Some("assets/models/triangle.glb".into()),
    );
    let text = ron::ser::to_string(&refs.to_value()).expect("serialize");
    let back: ReflectValue = ron::from_str(&text).expect("parse");
    let mut restored = AssetRefs::default();
    restored.apply(&back).expect("apply");
    assert_eq!(restored.get("MeshHandle").map(|r| r.kind), Some(AssetKind::Mesh));
    assert_eq!(
        restored.get("MeshHandle").and_then(|r| r.guid.as_deref()),
        Some("00000000000000000000000000001002")
    );
}

#[test]
fn unknown_fields_on_asset_refs_are_ignored() {
    let value = ReflectValue::Struct {
        type_name: "AssetRefs".into(),
        fields: vec![
            (
                "refs".into(),
                ReflectValue::List(vec![ReflectValue::Struct {
                    type_name: "MeshHandle".into(),
                    fields: vec![
                        (
                            "guid".into(),
                            ReflectValue::String("00000000000000000000000000009999".into()),
                        ),
                        (
                            "path".into(),
                            ReflectValue::String("models/x.glb".into()),
                        ),
                        ("kind".into(), ReflectValue::String("Mesh".into())),
                        ("future".into(), ReflectValue::I64(1)),
                    ],
                }]),
            ),
            ("extra".into(), ReflectValue::Bool(true)),
        ],
    };
    let mut refs = AssetRefs::default();
    refs.apply(&value).expect("apply");
    assert_eq!(refs.0.len(), 1);
    assert_eq!(refs.0[0].kind, AssetKind::Mesh);
}

#[test]
fn asset_ref_display_shows_guid() {
    let item = AssetRef {
        component: "MaterialHandle".into(),
        guid: Some("abcd".into()),
        path: None,
        kind: AssetKind::Material,
    };
    assert_eq!(item.to_string(), "abcd");
}

#[test]
fn asset_refs_restore_through_world_registry() {
    use redox_reflect::TypeRegistry;

    let mut registry = TypeRegistry::new();
    registry.register::<AssetRefs>();
    let mut world = World::new();
    let entity = world.spawn();
    let mut refs = AssetRefs::default();
    refs.set(
        "MaterialHandle",
        Some("00000000000000000000000000001001".into()),
        Some("assets/textures/a.png".into()),
    );
    world.add_component(entity, refs.clone());
    let text = registry.to_ron(&world).expect("to_ron");
    let mut loaded = World::new();
    registry.from_ron(&mut loaded, &text).expect("from_ron");
    let found = loaded.all_entities().next().expect("entity");
    let saved = loaded.get_component::<AssetRefs>(found).expect("asset refs");
    assert_eq!(saved, &refs);
}