//! Switching a character between animation and ragdoll, and feeding the ragdoll's bone poses to
//! skinning.
//!
//! While a character is ragdolled it has a [`RagdollRig`] and no [`CharacterController`]. The
//! rig holds the bone matrices last read from the bodies, in template order; [`RagdollRig::skin_matrices`]
//! turns them into the skinning matrices a [`redox_anim::Skeleton`] expects, which the caller
//! passes to `SkinnedMeshBinding::update`.

use redox_anim::Skeleton;
use redox_ecs::{Entity, World};
use redox_math::{Mat4, Vec3};
use redox_physics::{
    CharacterController, PhysicsContext, RagdollBody, RagdollError, RagdollHandle, RagdollTemplate,
    bone_world_from_body,
};

/// Whether a character follows its animation or lies as a ragdoll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterPoseMode {
    /// Bones come from the animator and the character controller moves the character.
    Animated,
    /// Bones come from the ragdoll bodies; the character controller is off.
    Ragdoll,
}

/// Ragdoll state of a character in [`CharacterPoseMode::Ragdoll`].
#[derive(Clone, Debug)]
pub struct RagdollRig {
    /// The spawned bodies.
    pub ragdoll: RagdollHandle,
    /// Skeleton bone index of each template bone, in template order.
    pub skeleton_of_template: Vec<usize>,
    /// Inverse bind matrix of each skeleton bone.
    pub inverse_bind: Vec<Mat4>,
    /// Model-space bone matrices in template order, from the bodies at the last refresh.
    pub bone_world: Vec<Mat4>,
    /// The character controller removed on entering the ragdoll, restored on exit.
    pub saved_controller: Option<CharacterController>,
}

impl RagdollRig {
    /// Skinning matrices in skeleton order (`bone_world * inverse_bind`).
    ///
    /// Bones the template does not cover keep the identity matrix, which is their bind pose.
    pub fn skin_matrices(&self) -> Vec<Mat4> {
        let mut out = vec![Mat4::IDENTITY; self.inverse_bind.len()];
        for (template, &bone) in self.skeleton_of_template.iter().enumerate() {
            if let (Some(world), Some(inverse)) =
                (self.bone_world.get(template), self.inverse_bind.get(bone))
            {
                out[bone] = *world * *inverse;
            }
        }
        out
    }

    /// Reads each body's pose from the physics world into `bone_world`. Bodies that are gone keep
    /// their last matrix.
    fn refresh(&mut self, physics: &PhysicsContext) {
        let offsets = self.ragdoll.offsets().to_vec();
        for (index, entity) in self.ragdoll.bodies().iter().enumerate() {
            let Some(handle) = physics.get_body_handle(*entity) else {
                continue;
            };
            let Some((translation, rotation)) = physics.get_rigid_body_transform(handle) else {
                continue;
            };
            let body = Mat4::from_rotation_translation(rotation, translation);
            if let (Some(slot), Some(offset)) = (self.bone_world.get_mut(index), offsets.get(index))
            {
                *slot = bone_world_from_body(body, *offset);
            }
        }
    }
}

/// Puts `entity` into ragdoll mode at the given pose, with the bodies starting at `model_world`.
///
/// `model_world` holds the character's bone matrices in world space, in skeleton order, so the
/// first ragdoll frame matches the last animated frame. The character controller is removed; the
/// character's own kinematic body is not removed here, so it stays in the physics world.
///
/// # Errors
/// Returns the template or physics error when the ragdoll cannot be spawned, or
/// [`RagdollError::MissingBone`] when a template bone is not in the skeleton.
pub fn enter_ragdoll(
    physics: &mut PhysicsContext,
    world: &mut World,
    entity: Entity,
    skeleton: &Skeleton,
    template: &RagdollTemplate,
    model_world: &[Mat4],
    root_velocity: Vec3,
    group: u32,
) -> Result<(), RagdollError> {
    let poses = template.bone_poses(skeleton, model_world)?;
    let skeleton_of_template = template
        .bones
        .iter()
        .map(|bone| {
            skeleton
                .bone_index(&bone.bone)
                .ok_or_else(|| RagdollError::MissingBone(bone.bone.clone()))
        })
        .collect::<Result<Vec<usize>, RagdollError>>()?;
    let ragdoll = physics.spawn_ragdoll(world, template, &poses, root_velocity, group)?;
    let saved_controller = world.get_component::<CharacterController>(entity).cloned();
    world.remove_component::<CharacterController>(entity);
    let rig = RagdollRig {
        ragdoll,
        skeleton_of_template,
        inverse_bind: skeleton.bones.iter().map(|b| b.inv_bind).collect(),
        bone_world: poses,
        saved_controller,
    };
    world.add_component(entity, rig);
    world.add_component(entity, CharacterPoseMode::Ragdoll);
    Ok(())
}

/// Returns `entity` to animation: removes the bodies and the rig, restores the controller.
///
/// Does nothing when the entity is not ragdolled. The bones snap back to the animator's pose
/// on the next frame; there is no blend.
pub fn exit_ragdoll(physics: &mut PhysicsContext, world: &mut World, entity: Entity) {
    let Some(rig) = world.get_component::<RagdollRig>(entity).cloned() else {
        return;
    };
    physics.remove_ragdoll(&rig.ragdoll);
    for body in rig.ragdoll.bodies() {
        world.remove_component::<RagdollBody>(*body);
    }
    world.remove_component::<RagdollRig>(entity);
    if let Some(controller) = rig.saved_controller {
        world.add_component(entity, controller);
    }
    world.add_component(entity, CharacterPoseMode::Animated);
}

/// Reads the bodies of every ragdolled character into its rig. Runs after the physics step.
///
/// Does nothing when the world has no [`PhysicsContext`].
pub fn ragdoll_pose_system(world: &mut World) {
    let Some(physics) = world.remove_resource::<PhysicsContext>() else {
        return;
    };
    let rigs: Vec<Entity> = world
        .query::<(Entity, &RagdollRig)>()
        .iter(world)
        .map(|(entity, _)| entity)
        .collect();
    for entity in rigs {
        if let Some(rig) = world.get_component_mut::<RagdollRig>(entity) {
            rig.refresh(&physics);
        }
    }
    world.insert_resource(physics);
}
