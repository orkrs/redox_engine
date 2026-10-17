use crossbeam_queue::SegQueue;
use rapier3d::prelude::*;
use redox_ecs::Entity;
use redox_ecs::world::World;
use redox_math::{Quat, Vec3};
use std::collections::{HashMap, HashSet};

use crate::raycast::{RaycastRequest, RaycastResult};

/// Errors of the physics layer.
#[derive(Debug, thiserror::Error)]
pub enum PhysicsError {
    /// The entity has no rigid body registered with the physics world.
    #[error("Entity not found in the physics mapping")]
    EntityNotFound,
}

/// The main physics context owning the rapier3d world and ECS mapping.
pub struct PhysicsContext {
    /// All rigid bodies.
    pub rigid_bodies: RigidBodySet,
    /// All colliders.
    pub colliders: ColliderSet,
    /// The rapier simulation pipeline.
    pub physics_pipeline: PhysicsPipeline,
    /// Sleeping/awake island bookkeeping.
    pub island_manager: IslandManager,
    /// Broad phase; also the source of scene queries.
    pub broad_phase: BroadPhaseBvh,
    /// Narrow phase (contact generation).
    pub narrow_phase: NarrowPhase,
    /// Impulse-based joints.
    pub impulse_joint_set: ImpulseJointSet,
    /// Multibody (reduced-coordinate) joints.
    pub multibody_joint_set: MultibodyJointSet,
    /// Continuous collision detection solver.
    pub ccd_solver: CCDSolver,
    /// World gravity vector.
    pub gravity: Vec3,
    /// Time step and solver parameters.
    pub integration_parameters: IntegrationParameters,
    /// Soft bodies (unused by the engine, required by the rapier API).
    pub soft_bodies: SoftBodySet,

    // Mappings between ECS Entities and Rapier Handles
    entity_to_body: HashMap<Entity, (RigidBodyHandle, Vec<ColliderHandle>)>,
    collider_to_entity: HashMap<ColliderHandle, Entity>,

    // Queues for systems
    /// Raycasts queued by gameplay code, answered by the next physics tick.
    pub raycast_requests: SegQueue<RaycastRequest>,
    collision_events: std::sync::Mutex<Vec<CollisionEvent>>,
}

impl PhysicsContext {
    /// Creates a new physics context with the given gravity.
    pub fn new(gravity: Vec3) -> Self {
        Self {
            rigid_bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            gravity,
            integration_parameters: IntegrationParameters::default(),
            soft_bodies: SoftBodySet::new(),
            entity_to_body: HashMap::new(),
            collider_to_entity: HashMap::new(),
            raycast_requests: SegQueue::new(),
            collision_events: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Advances the physics simulation by one step.
    pub fn step(&mut self, dt: f32) {
        self.integration_parameters.dt = dt;

        struct CollisionSink<'a> {
            buf: &'a std::sync::Mutex<Vec<CollisionEvent>>,
        }
        impl EventHandler for CollisionSink<'_> {
            fn handle_collision_event(
                &self,
                _bodies: &RigidBodySet,
                _colliders: &ColliderSet,
                event: CollisionEvent,
                _pair: Option<&ContactPair>,
            ) {
                self.buf
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(event);
            }
            fn handle_contact_force_event(
                &self,
                _dt: Real,
                _bodies: &RigidBodySet,
                _colliders: &ColliderSet,
                _pair: &ContactPair,
                _magnitude: Real,
            ) {
            }
            fn handle_soft_body_tear_event(&self, _: &SoftBodySet, _: &SoftBodyTearEvent) {}
        }

        let sink = CollisionSink {
            buf: &self.collision_events,
        };
        self.physics_pipeline.step(
            self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_bodies,
            &mut self.colliders,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.soft_bodies,
            &mut self.ccd_solver,
            &(),
            &sink,
        );
    }

    /// Drains and maps Rapier collision events without requiring a World borrow.
    pub fn drain_trigger_events(&mut self) -> Vec<(bool, crate::trigger::TriggerEnter)> {
        let raw: Vec<CollisionEvent> = self
            .collision_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect();
        raw.into_iter()
            .filter_map(|event| {
                crate::trigger::map_collision_event(
                    event,
                    |h| self.collider_to_entity.get(&h).copied(),
                    |h| self.colliders.get(h).is_some_and(|c| c.is_sensor()),
                )
            })
            .collect()
    }

    /// Drains Rapier collision events into ECS trigger enter/exit queues.
    pub fn emit_trigger_events(&mut self, world: &mut World) {
        let events = self.drain_trigger_events();
        for (started, enter) in events {
            if started {
                if let Some(queue) =
                    world.get_resource_mut::<redox_ecs::Events<crate::trigger::TriggerEnter>>()
                {
                    queue.send(enter);
                }
            } else if let Some(queue) =
                world.get_resource_mut::<redox_ecs::Events<crate::trigger::TriggerExit>>()
            {
                queue.send(crate::trigger::TriggerExit {
                    sensor: enter.sensor,
                    other: enter.other,
                });
            }
        }
    }

    /// Adds a sensor (trigger) collider on a kinematic body.
    pub fn add_trigger(
        &mut self,
        entity: Entity,
        translation: Vec3,
        builder: ColliderBuilder,
    ) -> Result<(RigidBodyHandle, ColliderHandle), PhysicsError> {
        let body = RigidBodyBuilder::kinematic_position_based().translation(translation);
        let col = builder
            .sensor(true)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .active_collision_types(ActiveCollisionTypes::all());
        let (bh, cols) = self.add_rigid_body(entity, body, vec![col])?;
        Ok((bh, cols[0]))
    }

    /// Adds a rigid body and its colliders to the world, establishing the ECS mapping.
    pub fn add_rigid_body(
        &mut self,
        entity: Entity,
        body_builder: RigidBodyBuilder,
        collider_builders: Vec<ColliderBuilder>,
    ) -> Result<(RigidBodyHandle, Vec<ColliderHandle>), PhysicsError> {
        let body_handle = self.rigid_bodies.insert(body_builder);
        let mut col_handles = Vec::with_capacity(collider_builders.len());

        for col_builder in collider_builders {
            let col_handle =
                self.colliders
                    .insert_with_parent(col_builder, body_handle, &mut self.rigid_bodies);
            col_handles.push(col_handle);
            self.collider_to_entity.insert(col_handle, entity);
        }

        self.entity_to_body
            .insert(entity, (body_handle, col_handles.clone()));
        log::debug!("Added rigid body for entity {:?}", entity);
        Ok((body_handle, col_handles))
    }

    /// Removes a rigid body and its colliders from the world.
    pub fn remove_rigid_body(&mut self, entity: Entity) -> Result<(), PhysicsError> {
        if let Some((body_handle, col_handles)) = self.entity_to_body.remove(&entity) {
            for col in col_handles {
                self.collider_to_entity.remove(&col);
            }
            self.rigid_bodies.remove(
                body_handle,
                &mut self.island_manager,
                &mut self.colliders,
                &mut self.impulse_joint_set,
                &mut self.multibody_joint_set,
                &mut self.soft_bodies,
                true,
            );
            log::debug!("Removed rigid body for entity {:?}", entity);
            Ok(())
        } else {
            Err(PhysicsError::EntityNotFound)
        }
    }

    /// Scene-query view over the current broad phase, bodies and colliders.
    pub fn query_pipeline<'a>(&'a self, filter: QueryFilter<'a>) -> QueryPipeline<'a> {
        self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.rigid_bodies,
            &self.colliders,
            filter,
        )
    }

    /// Retrieves the rigid body handle associated with the entity.
    pub fn get_body_handle(&self, entity: Entity) -> Option<RigidBodyHandle> {
        self.entity_to_body.get(&entity).map(|(h, _)| *h)
    }

    /// Teleports the body to the given pose and wakes it.
    pub fn set_rigid_body_transform(
        &mut self,
        handle: RigidBodyHandle,
        translation: Vec3,
        rotation: Quat,
    ) {
        if let Some(body) = self.rigid_bodies.get_mut(handle) {
            body.set_position(Pose::from_parts(translation, rotation), true);
        }
    }

    /// Sets the body's linear and angular velocity and wakes it.
    pub fn set_rigid_body_velocity(&mut self, handle: RigidBodyHandle, linvel: Vec3, angvel: Vec3) {
        if let Some(body) = self.rigid_bodies.get_mut(handle) {
            body.set_linvel(linvel, true);
            body.set_angvel(angvel, true);
        }
    }

    /// Returns the body's pose as translation and rotation.
    pub fn get_rigid_body_transform(&self, handle: RigidBodyHandle) -> Option<(Vec3, Quat)> {
        self.rigid_bodies.get(handle).map(|body| {
            let pos = body.position();
            (pos.translation, pos.rotation)
        })
    }

    /// Queues a raycast; its result arrives as a [`RaycastResult`] event.
    pub fn push_raycast_request(&self, request: RaycastRequest) {
        self.raycast_requests.push(request);
    }

    /// Processes and drains all queued raycast requests without requiring a World borrow.
    pub fn drain_raycast_results(&mut self) -> Vec<RaycastResult> {
        let mut results = Vec::new();
        while let Some(req) = self.raycast_requests.pop() {
            let ray = Ray::new(req.origin, req.direction);

            let hit = self
                .query_pipeline(QueryFilter::default())
                .cast_ray_and_get_normal(&ray, req.max_distance, true);
            let result = if let Some((handle, intersection)) = hit {
                let hit_point = req.origin + req.direction * intersection.time_of_impact;
                let mut normal = intersection.normal;
                if normal.dot(req.direction) > 0.0 {
                    normal = -normal;
                }
                let entity = self.collider_to_entity.get(&handle).copied();
                RaycastResult {
                    request_id: req.request_id,
                    hit: true,
                    entity,
                    point: hit_point,
                    normal,
                    distance: intersection.time_of_impact,
                }
            } else {
                RaycastResult {
                    request_id: req.request_id,
                    hit: false,
                    entity: None,
                    point: req.origin,
                    normal: Vec3::ZERO,
                    distance: req.max_distance,
                }
            };
            results.push(result);
        }
        results
    }

    /// Processes all queued raycast requests and sends them to the World's event queue.
    pub fn process_raycast_requests(&mut self, world: &mut World) {
        let results = self.drain_raycast_results();
        if let Some(events) = world.get_resource_mut::<redox_ecs::Events<RaycastResult>>() {
            for res in results {
                events.send(res);
            }
        }
    }

    /// Returns the collider handles attached to the entity's body, if it has one.
    pub(crate) fn collider_handles(&self, entity: Entity) -> Option<&[ColliderHandle]> {
        self.entity_to_body
            .get(&entity)
            .map(|(_, cols)| cols.as_slice())
    }

    /// Returns the entity owning the given collider, if it is mapped.
    pub(crate) fn entity_of_collider(&self, handle: ColliderHandle) -> Option<Entity> {
        self.collider_to_entity.get(&handle).copied()
    }

    /// Collects the colliders of every excluded entity, for use in query predicates.
    pub(crate) fn excluded_colliders(&self, exclude: &[Entity]) -> HashSet<ColliderHandle> {
        exclude
            .iter()
            .filter_map(|e| self.collider_handles(*e))
            .flatten()
            .copied()
            .collect()
    }

    /// Returns iterator over all registered entities and their bodies.
    pub fn active_bodies(&self) -> impl Iterator<Item = (&Entity, &RigidBodyHandle)> {
        self.entity_to_body.iter().map(|(e, (h, _))| (e, h))
    }

    /// Sync raycast with hit point + world normal. Updates the query pipeline first.
    /// Returns `(point, normal, entity)`.
    pub fn cast_ray_with_normal(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        exclude_entities: &[Entity],
    ) -> Option<(Vec3, Vec3, Option<Entity>)> {
        let dir_len = direction.length();
        if dir_len < 1e-6 {
            return None;
        }
        let direction = direction / dir_len;
        let ray = Ray::new(origin, direction);
        let excluded = self.excluded_colliders(exclude_entities);
        let predicate = |handle: ColliderHandle, _: &Collider| !excluded.contains(&handle);
        let filter = QueryFilter::default().predicate(&predicate);
        let (handle, intersection) =
            self.query_pipeline(filter)
                .cast_ray_and_get_normal(&ray, max_distance, true)?;
        let point = origin + direction * intersection.time_of_impact;
        let mut normal = intersection.normal;
        if normal.dot(direction) > 0.0 {
            normal = -normal;
        }
        let entity = self.collider_to_entity.get(&handle).copied();
        Some((point, normal, entity))
    }

    /// Performs a raycast from `origin` toward `target` (direction = (target - origin).normalized()),
    /// ignoring colliders attached to any of `exclude_entities`.
    ///
    /// Used by the audio occlusion system to test line-of-sight between listener and emitter
    /// without hitting the listener or emitter entities.
    ///
    /// Returns `Some((toi, hit_entity))` for the first hit, or `None` if the ray is clear.
    /// `toi` is the time-of-impact (distance along the ray from origin).
    pub fn cast_ray_excluding(
        &self,
        origin: Vec3,
        target: Vec3,
        exclude_entities: &[Entity],
    ) -> Option<(f32, Entity)> {
        let to_origin = target - origin;
        let distance = to_origin.length();
        if distance < 1e-6 {
            return None;
        }
        let direction = to_origin / distance;
        let ray = Ray::new(origin, direction);

        let excluded = self.excluded_colliders(exclude_entities);

        let predicate = |handle: ColliderHandle, _: &Collider| !excluded.contains(&handle);
        let filter = QueryFilter::default().predicate(&predicate);

        if let Some((handle, toi)) = self.query_pipeline(filter).cast_ray(&ray, distance, true) {
            self.collider_to_entity
                .get(&handle)
                .copied()
                .map(|entity| (toi, entity))
        } else {
            None
        }
    }
}
