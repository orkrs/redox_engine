//! Specification (tests first) for the collision layer configuration file (plan item B1).
//!
//! API to implement in `redox_physics` (new module `layer_config.rs`, re-exported from the crate root;
//! add `serde = { workspace = true }` and `ron = { workspace = true }` to the crate's dependencies):
//!
//! ```text
//! pub struct LayerConfig;
//! #[derive(Debug, PartialEq)]
//! pub enum LayerConfigError { Parse(String), DuplicateLayer(String), UnknownLayer(String), TooManyLayers(usize) }
//! impl LayerConfig {
//!     /// RON: `(layers: ["default", "player", ...], collide: [("player", "enemy"), ...])`.
//!     /// Layer `i` in the list is bit `1 << i`; at most 32 layers. Each `collide` pair makes the two layers
//!     /// interact with each other (symmetric); a layer may be paired with itself. A layer with no pair
//!     /// collides with nothing.
//!     pub fn parse(ron_text: &str) -> Result<Self, LayerConfigError>;
//!     pub fn len(&self) -> usize;
//!     pub fn is_empty(&self) -> bool;
//!     /// The single bit of a layer.
//!     pub fn bit(&self, name: &str) -> Option<u32>;
//!     /// Membership = the layer's bit, filter = the bits of every layer it is paired with.
//!     pub fn layers_of(&self, name: &str) -> Option<CollisionLayers>;
//!     /// Query mask for `cast_shape` / `overlap_*`: the OR of the named layers' bits.
//!     /// `None` when any name is unknown.
//!     pub fn mask(&self, names: &[&str]) -> Option<u32>;
//! }
//! ```
//!
//! Errors: text that is not valid RON of that shape -> `Parse`; the same name twice in `layers` ->
//! `DuplicateLayer(name)`; a pair naming a layer that is not listed -> `UnknownLayer(name)`; more than 32
//! layers -> `TooManyLayers(count)`.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::{Quat, Vec3};
use redox_physics::{LayerConfig, LayerConfigError, PhysicsContext, QueryShape};

const CONFIG: &str = r#"(
    layers: ["default", "player", "enemy", "ghost"],
    collide: [("default", "player"), ("default", "enemy"), ("player", "enemy")],
)"#;

fn config() -> LayerConfig {
    LayerConfig::parse(CONFIG).unwrap()
}

#[test]
fn layers_get_consecutive_bits_in_declaration_order() {
    let c = config();
    assert_eq!(c.len(), 4);
    assert!(!c.is_empty());
    assert_eq!(c.bit("default"), Some(0b0001));
    assert_eq!(c.bit("player"), Some(0b0010));
    assert_eq!(c.bit("enemy"), Some(0b0100));
    assert_eq!(c.bit("ghost"), Some(0b1000));
    assert_eq!(c.bit("nope"), None);
}

#[test]
fn pairs_are_symmetric_and_unpaired_layers_collide_with_nothing() {
    let c = config();
    let player = c.layers_of("player").unwrap();
    assert_eq!(player.member_of, 0b0010);
    assert_eq!(player.collides_with, 0b0001 | 0b0100, "default and enemy");
    let enemy = c.layers_of("enemy").unwrap();
    assert_eq!(enemy.collides_with, 0b0001 | 0b0010, "default and player");
    let ghost = c.layers_of("ghost").unwrap();
    assert_eq!(ghost.member_of, 0b1000);
    assert_eq!(ghost.collides_with, 0);
    assert!(c.layers_of("nope").is_none());
}

#[test]
fn a_layer_can_collide_with_itself() {
    let c = LayerConfig::parse(r#"(layers: ["a", "b"], collide: [("a", "a")])"#).unwrap();
    assert_eq!(c.layers_of("a").unwrap().collides_with, 0b01);
    assert_eq!(c.layers_of("b").unwrap().collides_with, 0);
}

#[test]
fn query_masks_or_the_named_bits() {
    let c = config();
    assert_eq!(c.mask(&["player", "enemy"]), Some(0b0110));
    assert_eq!(c.mask(&[]), Some(0));
    assert_eq!(c.mask(&["player", "nope"]), None);
}

#[test]
fn bad_configurations_report_a_specific_error() {
    assert!(matches!(
        LayerConfig::parse("not ron"),
        Err(LayerConfigError::Parse(_))
    ));
    assert_eq!(
        LayerConfig::parse(r#"(layers: ["a", "a"], collide: [])"#).err(),
        Some(LayerConfigError::DuplicateLayer("a".into()))
    );
    assert_eq!(
        LayerConfig::parse(r#"(layers: ["a"], collide: [("a", "zzz")])"#).err(),
        Some(LayerConfigError::UnknownLayer("zzz".into()))
    );
    let many: Vec<String> = (0..33).map(|i| format!("\"l{i}\"")).collect();
    let text = format!("(layers: [{}], collide: [])", many.join(","));
    assert_eq!(
        LayerConfig::parse(&text).err(),
        Some(LayerConfigError::TooManyLayers(33))
    );
    // Exactly 32 is fine.
    let ok: Vec<String> = (0..32).map(|i| format!("\"l{i}\"")).collect();
    assert!(LayerConfig::parse(&format!("(layers: [{}], collide: [])", ok.join(","))).is_ok());
}

#[test]
fn empty_configurations_are_valid() {
    let c = LayerConfig::parse("(layers: [], collide: [])").unwrap();
    assert!(c.is_empty());
}

/// Behaviour in a scene: a ghost falls through the default floor, a player lands on it, and a
/// query mask finds only the layers it names.
#[test]
fn layers_decide_who_collides_in_the_scene() {
    let c = config();
    let mut world = World::new();
    let floor = world.spawn();
    let player = world.spawn();
    let ghost = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(50.0, 0.5, 50.0)],
    )
    .unwrap();
    let (player_body, _) = ctx
        .add_rigid_body(
            player,
            RigidBodyBuilder::dynamic().translation(Vec3::new(-2.0, 2.0, 0.0)),
            vec![ColliderBuilder::ball(0.5)],
        )
        .unwrap();
    let (ghost_body, _) = ctx
        .add_rigid_body(
            ghost,
            RigidBodyBuilder::dynamic().translation(Vec3::new(2.0, 2.0, 0.0)),
            vec![ColliderBuilder::ball(0.5)],
        )
        .unwrap();
    ctx.set_collider_layers(floor, c.layers_of("default").unwrap())
        .unwrap();
    ctx.set_collider_layers(player, c.layers_of("player").unwrap())
        .unwrap();
    ctx.set_collider_layers(ghost, c.layers_of("ghost").unwrap())
        .unwrap();

    for _ in 0..180 {
        ctx.step(1.0 / 60.0);
    }
    let player_y = ctx.get_rigid_body_transform(player_body).unwrap().0.y;
    let ghost_y = ctx.get_rigid_body_transform(ghost_body).unwrap().0.y;
    assert!(
        (player_y - 0.5).abs() < 0.1,
        "player rests on the floor: {player_y}"
    );
    assert!(ghost_y < -2.0, "ghost fell through: {ghost_y}");

    // Query masks use the named layer bits.
    let enemies_only = c.mask(&["enemy"]).unwrap();
    assert!(
        !ctx.overlap_any(
            QueryShape::Sphere { radius: 100.0 },
            Vec3::ZERO,
            Quat::IDENTITY,
            &[],
            enemies_only
        ),
        "nothing is on the enemy layer"
    );
    let players_only = c.mask(&["player"]).unwrap();
    assert_eq!(
        ctx.overlap_shape(
            QueryShape::Sphere { radius: 100.0 },
            Vec3::ZERO,
            Quat::IDENTITY,
            &[],
            players_only
        ),
        vec![player]
    );
}
