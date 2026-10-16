//! Gameplay Ability System (GAS-lite) + StateTree AI/game-flow.
//!
//! Uplift v2 Phase 8: attributes, abilities, timed effects, and hierarchical
//! StateTree for sentinels / game phases.

pub mod state_tree;

use redox_ecs::{Query, World};
use redox_math::Vec3;

pub use state_tree::{
    sentinel_patrol_chase, sentinel_patrol_chase_ranges, state_tree_system, StateId, StateNode,
    StateTickResult, StateTransition, StateTree, StateTreeBuilder, StateTreeComponent,
    StateTreeContext, TransitionCondition,
};

/// Named scalar attribute (health, stamina, etc.).
#[derive(Clone, Copy, Debug, Default)]
pub struct Attribute {
    pub current: f32,
    pub max: f32,
}

impl Attribute {
    pub fn new(max: f32) -> Self {
        Self { current: max, max }
    }

    pub fn clamp(&mut self) {
        self.current = self.current.clamp(0.0, self.max);
    }
}

/// Component holding a set of gameplay attributes keyed by name.
#[derive(Clone, Debug, Default)]
pub struct AttributeSet {
    pub health: Attribute,
    pub stamina: Attribute,
}

impl AttributeSet {
    pub fn default_character() -> Self {
        Self {
            health: Attribute::new(100.0),
            stamina: Attribute::new(100.0),
        }
    }
}

/// Lightweight ability descriptor attached to an entity.
#[derive(Clone, Debug)]
pub struct Ability {
    pub id: u32,
    pub cooldown: f32,
    pub remaining_cooldown: f32,
    pub cost_stamina: f32,
}

impl Ability {
    pub fn new(id: u32, cooldown: f32, cost_stamina: f32) -> Self {
        Self {
            id,
            cooldown,
            remaining_cooldown: 0.0,
            cost_stamina,
        }
    }

    pub fn ready(&self) -> bool {
        self.remaining_cooldown <= 0.0
    }
}

/// Modifier applied over time (DOT, buff, debuff).
#[derive(Clone, Debug)]
pub struct GameplayEffect {
    pub duration: f32,
    pub remaining: f32,
    pub health_delta_per_sec: f32,
    pub stamina_delta_per_sec: f32,
}

impl GameplayEffect {
    pub fn instant_heal(amount: f32) -> Self {
        Self {
            duration: 0.0,
            remaining: 0.0,
            health_delta_per_sec: amount,
            stamina_delta_per_sec: 0.0,
        }
    }

    pub fn dot(dps: f32, duration: f32) -> Self {
        Self {
            duration,
            remaining: duration,
            health_delta_per_sec: -dps,
            stamina_delta_per_sec: 0.0,
        }
    }

    pub fn active(&self) -> bool {
        self.remaining > 0.0 || self.duration == 0.0
    }
}

/// Component listing active effects on an entity.
#[derive(Clone, Debug, Default)]
pub struct ActiveEffects {
    pub effects: Vec<GameplayEffect>,
}

/// Advances ability cooldowns each frame.
pub fn ability_cooldown_system(world: &mut World, delta_time: f32) {
    for ability in Query::<&mut Ability>::new().iter_mut(world) {
        if ability.remaining_cooldown > 0.0 {
            ability.remaining_cooldown = (ability.remaining_cooldown - delta_time).max(0.0);
        }
    }
}

/// Applies active gameplay effects to attribute sets.
pub fn gameplay_effect_system(world: &mut World, delta_time: f32) {
    for (attrs, effects) in Query::<(&mut AttributeSet, &mut ActiveEffects)>::new().iter_mut(world)
    {
        let mut expired = Vec::new();
        for (i, effect) in effects.effects.iter_mut().enumerate() {
            if effect.duration > 0.0 {
                let dt = delta_time.min(effect.remaining);
                attrs.health.current += effect.health_delta_per_sec * dt;
                attrs.stamina.current += effect.stamina_delta_per_sec * dt;
                effect.remaining -= dt;
                if effect.remaining <= 0.0 {
                    expired.push(i);
                }
            } else {
                attrs.health.current += effect.health_delta_per_sec;
                attrs.stamina.current += effect.stamina_delta_per_sec;
                expired.push(i);
            }
        }
        attrs.health.clamp();
        attrs.stamina.clamp();
        for i in expired.into_iter().rev() {
            effects.effects.remove(i);
        }
    }
}

/// Triggers an ability if off cooldown and stamina is sufficient.
pub fn try_activate_ability(attrs: &mut AttributeSet, ability: &mut Ability) -> bool {
    if !ability.ready() || attrs.stamina.current < ability.cost_stamina {
        return false;
    }
    attrs.stamina.current -= ability.cost_stamina;
    ability.remaining_cooldown = ability.cooldown;
    true
}

/// World-space knockback impulse stub for gameplay events.
pub fn knockback_impulse(direction: Vec3, strength: f32) -> Vec3 {
    let len_sq = direction.length_squared();
    if len_sq < 1e-8 {
        return Vec3::ZERO;
    }
    direction * (strength / len_sq.sqrt())
}
