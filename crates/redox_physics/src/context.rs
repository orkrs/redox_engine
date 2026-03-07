use crossbeam_queue::SegQueue;
use rapier3d::prelude::*;
use redox_ecs::Entity;
use redox_ecs::world::World;
use redox_math::{Quat, Vec3};
use std::collections::HashMap;

use crate::raycast::{RaycastRequest, RaycastResult};
use crate::utils::{quat_from_rapier, quat_to_rapier, vec3_from_rapier, vec3_to_rapier};

#[derive(Debug, thiserror::Error)]
pub enum PhysicsError {
    #[error("Entity not found in the physics mapping")]
    EntityNotFound,
}

/// The main physics context owning the rapier3d world and ECS mapping.
pub struct PhysicsContext {
    pub rigid_bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub physics_pipeline: PhysicsPipeline,
    pub island_manager: IslandManager,
    pub broad_phase: BroadPhase,
    pub narrow_phase: NarrowPhase,
    pub impulse_joint_set: ImpulseJointSet,
    pub multibody_joint_set: MultibodyJointSet,
    pub ccd_solver: CCDSolver,
    pub gravity: Vector<f32>,
    pub integration_parameters: IntegrationParameters,
    pub query_pipeline: QueryPipeline,

    // Mappings between ECS Entities and Rapier Handles
    entity_to_body: HashMap<Entity, (RigidBodyHandle, Vec<ColliderHandle>)>,
    collider_to_entity: HashMap<ColliderHandle, Entity>,

    // Queues for systems
    pub raycast_requests: SegQueue<RaycastRequest>,
}

impl PhysicsContext {
    /// Creates a new physics context with the given gravity.
    pub fn new(gravity: Vec3) -> Self {
        Self {
            rigid_bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: BroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            gravity: vec3_to_rapier(gravity),
            integration_parameters: IntegrationParameters::default(),
            query_pipeline: QueryPipeline::new(),
            entity_to_body: HashMap::new(),
            collider_to_entity: HashMap::new(),
            raycast_requests: SegQueue::new(),
        }
    }

    /// Advances the physics simulation by one step.
    pub fn step(&mut self, dt: f32) {
        self.integration_parameters.dt = dt;

        self.physics_pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_bodies,
            &mut self.colliders,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(),
            &(),
        );
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
                true,
            );
            log::debug!("Removed rigid body for entity {:?}", entity);
            Ok(())
        } else {
            Err(PhysicsError::EntityNotFound)
        }
    }

    /// Retrieves the rigid body handle associated with the entity.
    pub fn get_body_handle(&self, entity: Entity) -> Option<RigidBodyHandle> {
        self.entity_to_body.get(&entity).map(|(h, _)| *h)
    }

    pub fn set_rigid_body_transform(
        &mut self,
        handle: RigidBodyHandle,
        translation: Vec3,
        rotation: Quat,
    ) {
        if let Some(body) = self.rigid_bodies.get_mut(handle) {
            body.set_position(
                Isometry::from_parts(
                    Translation::from(vec3_to_rapier(translation)),
                    quat_to_rapier(rotation),
                ),
                true,
            );
        }
    }

    pub fn set_rigid_body_velocity(&mut self, handle: RigidBodyHandle, linvel: Vec3, angvel: Vec3) {
        if let Some(body) = self.rigid_bodies.get_mut(handle) {
            body.set_linvel(vec3_to_rapier(linvel), true);
            body.set_angvel(vec3_to_rapier(angvel), true);
        }
    }

    pub fn get_rigid_body_transform(&self, handle: RigidBodyHandle) -> Option<(Vec3, Quat)> {
        self.rigid_bodies.get(handle).map(|body| {
            let pos = body.position();
            (
                vec3_from_rapier(&pos.translation.vector),
                quat_from_rapier(&pos.rotation),
            )
        })
    }

    pub fn push_raycast_request(&self, request: RaycastRequest) {
        self.raycast_requests.push(request);
    }

    /// Processes all queued raycast requests.
    pub fn process_raycast_requests(&mut self, _world: &mut World) {
        // Here we'd typically use something like world.get_resource_mut::<Events<RaycastResult>>()
        // For now, we process and log them or push to an internal processed queue.
        while let Some(req) = self.raycast_requests.pop() {
            let ray = Ray::new(
                Point::from(vec3_to_rapier(req.origin)),
                vec3_to_rapier(req.direction),
            );

            if let Some((handle, toi)) = self.query_pipeline.cast_ray(
                &self.rigid_bodies,
                &self.colliders,
                &ray,
                req.max_distance,
                true,
                QueryFilter::default(),
            ) {
                let hit_point = req.origin + req.direction * toi;
                let entity = self.collider_to_entity.get(&handle).copied();

                let _result = RaycastResult {
                    request_id: req.request_id,
                    hit: true,
                    entity,
                    point: hit_point,
                    normal: Vec3::new(0.0, 1.0, 0.0), // Need cast_ray_and_get_normal for actual normal
                    distance: toi,
                };

                // world.send_event(result); // Requires ECS event sending implementation
            }
        }
    }

    /// Returns iterator over all registered entities and their bodies.
    pub fn active_bodies(&self) -> impl Iterator<Item = (&Entity, &RigidBodyHandle)> {
        self.entity_to_body.iter().map(|(e, (h, _))| (e, h))
    }
}
