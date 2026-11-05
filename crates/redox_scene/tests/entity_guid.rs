//! `EntityGuid` text form, serde form and reflected value; `ParentRef` value.

use redox_reflect::{Reflect, ReflectValue};
use redox_scene::{EntityGuid, ParentRef};

const SAMPLE: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn display_and_from_str_roundtrip_without_loss() {
    let guid: EntityGuid = SAMPLE.parse().expect("valid guid");
    assert_eq!(guid.to_string(), SAMPLE);
    assert_eq!(guid.to_string().parse::<EntityGuid>().expect("parse"), guid);
}

#[test]
fn leading_zeros_are_kept_in_the_text() {
    let guid = EntityGuid(1);
    assert_eq!(guid.to_string(), "00000000000000000000000000000001");
    assert_eq!(
        "00000000000000000000000000000001".parse::<EntityGuid>(),
        Ok(EntityGuid(1))
    );
}

#[test]
fn text_that_is_not_32_hex_digits_is_rejected() {
    assert!("abc".parse::<EntityGuid>().is_err());
    assert!("0123456789abcdef0123456789abcdeg".parse::<EntityGuid>().is_err());
    assert!("0123456789abcdef0123456789abcdef00".parse::<EntityGuid>().is_err());
}

#[test]
fn random_guids_are_distinct() {
    let a = EntityGuid::new_random();
    let b = EntityGuid::new_random();
    assert_ne!(a, b);
}

#[test]
fn serde_form_is_the_hex_string() {
    let guid: EntityGuid = SAMPLE.parse().expect("valid guid");
    let text = ron::ser::to_string(&guid).expect("serialize");
    assert_eq!(text, format!("\"{SAMPLE}\""));
    let back: EntityGuid = ron::from_str(&text).expect("parse");
    assert_eq!(back, guid);
}

#[test]
fn entity_guid_value_roundtrips() {
    let guid: EntityGuid = SAMPLE.parse().expect("valid guid");
    let value = guid.to_value();
    let mut restored = EntityGuid(0);
    restored.apply(&value).expect("apply");
    assert_eq!(restored, guid);
}

#[test]
fn parent_ref_value_roundtrips_and_survives_ron() {
    let parent = ParentRef(SAMPLE.parse().expect("valid guid"));
    let value = parent.to_value();
    assert_eq!(ParentRef::from_value(&value), Some(parent));

    let text = ron::ser::to_string(&value).expect("serialize");
    let back: ReflectValue = ron::from_str(&text).expect("parse");
    assert_eq!(ParentRef::from_value(&back), Some(parent));
}

#[test]
fn parent_ref_ignores_a_value_with_a_bad_guid() {
    let value = ReflectValue::Struct {
        type_name: "ParentRef".into(),
        fields: vec![("parent".into(), ReflectValue::String("not-a-guid".into()))],
    };
    assert_eq!(ParentRef::from_value(&value), None);
}
