//! Contact enter/exit events between solid colliders, and per-entity physics materials.
//!
//! Contact events are opt-in per entity (see [`PhysicsContext::set_contact_events`]). Sensor
//! overlaps are not contacts; they keep flowing through the trigger path.

use rapier3d::geometry::{Collider, ColliderHandle, CollisionEvent, ContactManifold, ContactPair};
use rapier3d::prelude::ActiveEvents;
use redox_ecs::Entity;
use redox_math::Vec3;

use crate::context::{PhysicsContext, PhysicsError};

/// Two solid colliders started touching.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactEnter {
    /// One entity of the pair (order is unspecified).
    pub a: Entity,
    /// The other entity of the pair.
    pub b: Entity,
    /// World-space contact point.
    pub point: Vec3,
    /// Unit contact normal.
    pub normal: Vec3,
    /// Total normal impulse of the contact in the step in which it began.
    pub impulse: f32,
}

/// Two solid colliders stopped touching.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactExit {
    /// One entity of the pair (order is unspecified).
    pub a: Entity,
    /// The other entity of the pair.
    pub b: Entity,
}

/// A contact between two entities that started or stopped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContactEvent {
    /// The pair started touching.
    Enter(ContactEnter),
    /// The pair stopped touching.
    Exit(ContactExit),
}

/// Two solid colliders stayed in contact for one step (opt-in, see
/// [`PhysicsContext::set_contact_stay`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactStay {
    /// One entity of the pair (order is unspecified).
    pub a: Entity,
    /// The other entity of the pair.
    pub b: Entity,
    /// Total normal impulse of the pair's contacts in this step.
    pub impulse: f32,
}

/// Surface properties applied to every collider of an entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsMaterial {
    /// Coulomb friction coefficient (combined with the other surface by averaging).
    pub friction: f32,
    /// Bounciness: 0 absorbs the impact, 1 bounces back at the same speed.
    pub restitution: f32,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            friction: 0.5,
            restitution: 0.0,
        }
    }
}

/// Normal used when a started pair has no manifold to read it from.
const FALLBACK_NORMAL: Vec3 = Vec3::Y;

/// Order-independent key of an entity pair.
fn pair_key(a: Entity, b: Entity) -> (Entity, Entity) {
    if a <= b { (a, b) } else { (b, a) }
}

impl PhysicsContext {
    /// Opts the colliders of `entity` in or out of contact events.
    ///
    /// Events are produced for a pair when at least one of its colliders opted in. Sensor
    /// colliders always keep their collision events so that triggers keep working.
    pub fn set_contact_events(
        &mut self,
        entity: Entity,
        enabled: bool,
    ) -> Result<(), PhysicsError> {
        self.for_each_collider_of(entity, |collider| {
            let keep = enabled || collider.is_sensor();
            let flags = if keep {
                ActiveEvents::COLLISION_EVENTS
            } else {
                ActiveEvents::empty()
            };
            collider.set_active_events(flags);
        })
    }

    /// Takes all contact events collected since the last call, oldest first.
    pub fn drain_contact_events(&mut self) -> Vec<ContactEvent> {
        std::mem::take(&mut self.contact_events)
    }

    /// Opts `entity` in or out of per-step [`ContactStay`] reports. Independent of
    /// [`Self::set_contact_events`]; off by default.
    ///
    /// A report is produced for a pair while it touches, when at least one of its entities
    /// opted in.
    pub fn set_contact_stay(&mut self, entity: Entity, enabled: bool) -> Result<(), PhysicsError> {
        self.collider_handles(entity)
            .ok_or(PhysicsError::EntityNotFound)?;
        if enabled {
            self.contact_stay_entities.insert(entity);
        } else {
            self.contact_stay_entities.remove(&entity);
        }
        Ok(())
    }

    /// Takes all stay reports collected since the last call, oldest first.
    pub fn drain_contact_stays(&mut self) -> Vec<ContactStay> {
        std::mem::take(&mut self.contact_stays)
    }

    /// Records one [`ContactStay`] for every touching pair with an opted-in entity.
    ///
    /// Must run after the pipeline step, like [`Self::collect_contact_events`].
    pub(crate) fn collect_contact_stays(&mut self) {
        if self.contact_stay_entities.is_empty() {
            return;
        }
        let stays: Vec<ContactStay> = self
            .narrow_phase
            .contact_pairs()
            .filter_map(|pair| self.stay_of_pair(pair))
            .collect();
        self.contact_stays.extend(stays);
    }

    /// The stay report of `pair`, if it touches and one of its entities opted in.
    fn stay_of_pair(&self, pair: &ContactPair) -> Option<ContactStay> {
        if !pair.has_any_active_contact() {
            return None;
        }
        let a = self.entity_of_collider(pair.collider1)?;
        let b = self.entity_of_collider(pair.collider2)?;
        let opted_in =
            self.contact_stay_entities.contains(&a) || self.contact_stay_entities.contains(&b);
        opted_in.then(|| ContactStay {
            a,
            b,
            impulse: pair.total_impulse_magnitude(),
        })
    }

    /// Sets friction and restitution on every collider of `entity`.
    pub fn set_physics_material(
        &mut self,
        entity: Entity,
        material: PhysicsMaterial,
    ) -> Result<(), PhysicsError> {
        self.for_each_collider_of(entity, |collider| {
            collider.set_friction(material.friction);
            collider.set_restitution(material.restitution);
        })
    }

    /// Applies `update` to every collider attached to `entity`.
    fn for_each_collider_of(
        &mut self,
        entity: Entity,
        mut update: impl FnMut(&mut Collider),
    ) -> Result<(), PhysicsError> {
        let handles = self
            .collider_handles(entity)
            .ok_or(PhysicsError::EntityNotFound)?
            .to_vec();
        for handle in handles {
            if let Some(collider) = self.colliders.get_mut(handle) {
                update(collider);
            }
        }
        Ok(())
    }

    /// Sets the minimum impulse a contact `Enter` needs to be reported. Negative and NaN values
    /// count as 0.0. Applies to events produced after the call; `Stay` reports are unaffected.
    pub fn set_contact_impulse_threshold(&mut self, min_impulse: f32) {
        self.contact_impulse_min = if min_impulse.is_nan() || min_impulse < 0.0 {
            0.0
        } else {
            min_impulse
        };
    }

    /// The current contact impulse threshold (0.0 by default).
    pub fn contact_impulse_threshold(&self) -> f32 {
        self.contact_impulse_min
    }

    /// Converts the solid-collider events recorded during the last step into contact events.
    ///
    /// Must run after the pipeline step: the impulses of a started pair are only known once the
    /// solver has run.
    pub(crate) fn collect_contact_events(&mut self) {
        let raw: Vec<CollisionEvent> = self
            .contact_raw
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect();
        for event in raw {
            if let Some(contact) = self.map_contact_event(event) {
                self.contact_events.push(contact);
            }
        }
    }

    /// Maps one raw collision event to an entity-level contact event, if both colliders are known.
    ///
    /// An `Enter` below the impulse threshold is dropped and its pair remembered, so that the
    /// matching `Exit` is dropped as well.
    fn map_contact_event(&mut self, event: CollisionEvent) -> Option<ContactEvent> {
        let (c1, c2) = match event {
            CollisionEvent::Started(c1, c2, _) | CollisionEvent::Stopped(c1, c2, _) => (c1, c2),
        };
        let a = self.entity_of_collider(c1)?;
        let b = self.entity_of_collider(c2)?;
        let key = pair_key(a, b);
        match event {
            CollisionEvent::Started(..) => {
                let (point, normal, impulse) = self.contact_summary(c1, c2);
                if impulse < self.contact_impulse_min {
                    self.contact_suppressed_pairs.insert(key);
                    return None;
                }
                Some(ContactEvent::Enter(ContactEnter {
                    a,
                    b,
                    point,
                    normal,
                    impulse,
                }))
            }
            CollisionEvent::Stopped(..) => {
                if self.contact_suppressed_pairs.remove(&key) {
                    return None;
                }
                Some(ContactEvent::Exit(ContactExit { a, b }))
            }
        }
    }

    /// Point, normal and total impulse of the pair `(c1, c2)` as the narrow phase sees it now.
    ///
    /// Falls back to a zero point, an up normal and zero impulse when the pair has no contact.
    fn contact_summary(&self, c1: ColliderHandle, c2: ColliderHandle) -> (Vec3, Vec3, f32) {
        let Some(pair) = self.narrow_phase.contact_pair(c1, c2) else {
            return (Vec3::ZERO, FALLBACK_NORMAL, 0.0);
        };
        let impulse = pair.total_impulse_magnitude();
        let manifolds = pair.manifolds();
        let normal = manifolds
            .iter()
            .find(|m| !m.points.is_empty())
            .map_or(FALLBACK_NORMAL, |m| {
                m.data.normal.normalize_or(FALLBACK_NORMAL)
            });
        (self.mean_contact_point(c1, manifolds), normal, impulse)
    }

    /// Average world-space position of all manifold contact points of the collider `c1`.
    fn mean_contact_point(&self, c1: ColliderHandle, manifolds: &[ContactManifold]) -> Vec3 {
        let Some(pose) = self.colliders.get(c1).map(|c| *c.position()) else {
            return Vec3::ZERO;
        };
        let points: Vec<Vec3> = manifolds
            .iter()
            .flat_map(|m| m.points.iter())
            .map(|p| pose.transform_point(p.local_p1))
            .collect();
        if points.is_empty() {
            return Vec3::ZERO;
        }
        points.iter().copied().sum::<Vec3>() / points.len() as f32
    }
}
