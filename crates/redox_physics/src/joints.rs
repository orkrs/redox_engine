//! Joints between rigid bodies, with optional break force and break torque, and optional
//! `limits` on hinge rotation and slider travel.

use std::collections::HashMap;

use rapier3d::dynamics::ImpulseJoint;
use rapier3d::prelude::{
    FixedJointBuilder, GenericJoint, ImpulseJointHandle, PrismaticJointBuilder,
    RevoluteJointBuilder, RopeJointBuilder, SphericalJointBuilder, SpringJointBuilder,
};
use redox_ecs::Entity;
use redox_math::Vec3;

use crate::context::{PhysicsContext, PhysicsError};

/// The constraint a joint imposes on its two bodies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointKind {
    /// No relative motion.
    Fixed,
    /// Rotation about `axis` only (axis in the local space of both bodies).
    Hinge {
        /// Hinge axis.
        axis: Vec3,
    },
    /// Translation along `axis` only (axis in the local space of both bodies).
    Slider {
        /// Slide axis.
        axis: Vec3,
    },
    /// Free rotation about the anchor.
    Ball,
    /// Force-based spring: `F = stiffness * extension` (N/m).
    Spring {
        /// Length at which the spring exerts no force.
        rest_length: f32,
        /// Stiffness in N/m.
        stiffness: f32,
        /// Damping in N*s/m.
        damping: f32,
    },
    /// The anchors may not be farther apart than `max_length`.
    Rope {
        /// Maximum anchor distance in metres.
        max_length: f32,
    },
}

/// Description of a joint between two bodies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointDesc {
    /// The constraint.
    pub kind: JointKind,
    /// Anchor in the local space of body A.
    pub anchor_a: Vec3,
    /// Anchor in the local space of body B.
    pub anchor_b: Vec3,
    /// The joint is removed when its constraint force exceeds this value (N).
    pub break_force: Option<f32>,
    /// The joint is removed when its constraint torque exceeds this value (N*m).
    ///
    /// Checked on the angular part of the constraint impulse divided by the step `dt`, after
    /// each step. Either `break_force` or `break_torque` breaking the joint removes it.
    /// `None` (the default) means torque never breaks the joint.
    pub break_torque: Option<f32>,
    /// `(min, max)` limits: radians for `Hinge`, metres for `Slider`, measured from the pose in
    /// which the joint was created. Ignored for other kinds.
    ///
    /// `None` (the default) means no limits. `min > max` or a NaN bound also means no limits.
    pub limits: Option<(f32, f32)>,
}

impl JointDesc {
    /// Creates a description with zero anchors and no break force or break torque.
    pub fn new(kind: JointKind) -> Self {
        Self {
            kind,
            anchor_a: Vec3::ZERO,
            anchor_b: Vec3::ZERO,
            break_force: None,
            break_torque: None,
            limits: None,
        }
    }

    /// The configured limits as rapier wants them, or `None` when absent, reversed or NaN.
    fn valid_limits(&self) -> Option<[f32; 2]> {
        let (min, max) = self.limits?;
        if min.is_nan() || max.is_nan() || min > max {
            return None;
        }
        Some([min, max])
    }

    /// Builds the rapier joint; jointed bodies never collide with each other.
    fn to_generic(self) -> GenericJoint {
        let mut joint: GenericJoint = match self.kind {
            JointKind::Fixed => FixedJointBuilder::new().build().into(),
            JointKind::Hinge { axis } => {
                let mut builder = RevoluteJointBuilder::new(axis);
                if let Some(limits) = self.valid_limits() {
                    builder = builder.limits(limits);
                }
                builder.build().into()
            }
            JointKind::Slider { axis } => {
                let mut builder = PrismaticJointBuilder::new(axis);
                if let Some(limits) = self.valid_limits() {
                    builder = builder.limits(limits);
                }
                builder.build().into()
            }
            JointKind::Ball => SphericalJointBuilder::new().build().into(),
            JointKind::Spring {
                rest_length,
                stiffness,
                damping,
            } => SpringJointBuilder::new(rest_length, stiffness, damping)
                .build()
                .into(),
            JointKind::Rope { max_length } => RopeJointBuilder::new(max_length).build().into(),
        };
        joint
            .set_local_anchor1(self.anchor_a)
            .set_local_anchor2(self.anchor_b)
            .set_contacts_enabled(false);
        joint
    }
}

/// Opaque identifier of a joint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JointId(u64);

/// Event reporting that a joint exceeded its break force and was removed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointBroken {
    /// The joint that broke.
    pub joint: JointId,
    /// First attached entity.
    pub a: Entity,
    /// Second attached entity.
    pub b: Entity,
}

/// Engine-side record of one joint.
#[derive(Clone, Copy, Debug)]
struct JointRecord {
    handle: ImpulseJointHandle,
    a: Entity,
    b: Entity,
    break_force: Option<f32>,
    break_torque: Option<f32>,
}

/// Joint bookkeeping owned by the physics context.
#[derive(Default)]
pub(crate) struct JointBook {
    next_id: u64,
    records: HashMap<JointId, JointRecord>,
    broken: Vec<JointBroken>,
}

/// Euclidean magnitude of three impulse components.
fn magnitude3(c: &[f32]) -> f32 {
    (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
}

/// True when `value` is above `limit`; a missing limit is never exceeded.
fn above(value: f32, limit: Option<f32>) -> bool {
    limit.is_some_and(|limit| value > limit)
}

/// Force and torque a joint carried during the last step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct JointLoad {
    /// Magnitude of the linear constraint force (N).
    force: f32,
    /// Magnitude of the constraint torque about the joint anchor (N*m).
    torque: f32,
}

impl PhysicsContext {
    /// Joins the bodies of `a` and `b`.
    ///
    /// Returns [`PhysicsError::EntityNotFound`] when either entity has no body.
    pub fn add_joint(
        &mut self,
        a: Entity,
        b: Entity,
        desc: JointDesc,
    ) -> Result<JointId, PhysicsError> {
        let body_a = self
            .get_body_handle(a)
            .ok_or(PhysicsError::EntityNotFound)?;
        let body_b = self
            .get_body_handle(b)
            .ok_or(PhysicsError::EntityNotFound)?;
        let handle = self
            .impulse_joint_set
            .insert(body_a, body_b, desc.to_generic(), true);
        let id = JointId(self.joints.next_id);
        self.joints.next_id += 1;
        self.joints.records.insert(
            id,
            JointRecord {
                handle,
                a,
                b,
                break_force: desc.break_force,
                break_torque: desc.break_torque,
            },
        );
        Ok(id)
    }

    /// Removes a joint; returns false when it did not exist.
    pub fn remove_joint(&mut self, joint: JointId) -> bool {
        match self.joints.records.remove(&joint) {
            Some(record) => {
                self.impulse_joint_set.remove(record.handle, true);
                true
            }
            None => false,
        }
    }

    /// Returns true while the joint exists.
    pub fn joint_exists(&self, joint: JointId) -> bool {
        self.joints.records.contains_key(&joint)
    }

    /// Takes the joints that broke since the last call (oldest first).
    pub fn drain_joint_broken(&mut self) -> Vec<JointBroken> {
        std::mem::take(&mut self.joints.broken)
    }

    /// Forgets every joint attached to `entity` (rapier drops them with the body).
    pub(crate) fn forget_joints_of(&mut self, entity: Entity) {
        self.joints
            .records
            .retain(|_, r| r.a != entity && r.b != entity);
    }

    /// Removes joints whose constraint force or torque over the last step exceeded their limits.
    pub(crate) fn break_overloaded_joints(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let mut overloaded: Vec<JointId> = self
            .joints
            .records
            .iter()
            .filter(|(_, r)| self.exceeds_break_limits(r, dt))
            .map(|(id, _)| *id)
            .collect();
        overloaded.sort_by_key(|id| id.0);
        for id in overloaded {
            if let Some(record) = self.joints.records.get(&id).copied() {
                self.remove_joint(id);
                self.joints.broken.push(JointBroken {
                    joint: id,
                    a: record.a,
                    b: record.b,
                });
            }
        }
    }

    /// True when the joint's force exceeds `break_force` or its torque exceeds `break_torque`.
    fn exceeds_break_limits(&self, record: &JointRecord, dt: f32) -> bool {
        let load = self.joint_load(record, dt);
        above(load.force, record.break_force) || above(load.torque, record.break_torque)
    }

    /// Force and torque of a joint over the last step.
    ///
    /// rapier reports the impulse of the last solver substep, so force = impulse / (dt / substeps).
    /// The torque is the larger of the reported angular impulse and the moment the linear
    /// constraint force has about the joint anchor (rapier reports no angular impulse for a load
    /// that hangs off a lever, but the moment is real and is what tears the joint).
    fn joint_load(&self, record: &JointRecord, dt: f32) -> JointLoad {
        let Some(joint) = self.impulse_joint_set.get(record.handle) else {
            return JointLoad::default();
        };
        let substeps = self.integration_parameters.num_solver_iterations.max(1) as f32;
        let to_rate = substeps / dt;
        let impulse = Vec3::new(joint.impulses[0], joint.impulses[1], joint.impulses[2]);
        let angular = magnitude3(&joint.impulses[3..6]) * to_rate;
        let lever = self.lever_moment(joint, impulse) * to_rate;
        JointLoad {
            force: impulse.length() * to_rate,
            torque: angular.max(lever),
        }
    }

    /// Moment of the linear constraint impulse (given in the joint frame) about the bodies' centres
    /// of mass; the larger of the two bodies' moments.
    fn lever_moment(&self, joint: &ImpulseJoint, impulse: Vec3) -> f32 {
        let (Some(body1), Some(body2)) = (
            self.rigid_bodies.get(joint.body1()),
            self.rigid_bodies.get(joint.body2()),
        ) else {
            return 0.0;
        };
        let frame1 = *body1.position() * joint.data.local_frame1;
        let frame2 = *body2.position() * joint.data.local_frame2;
        let force = frame1.rotation * impulse;
        let anchor = frame2.translation;
        let moment = |centre: Vec3| (centre - anchor).cross(force).length();
        moment(body1.center_of_mass()).max(moment(body2.center_of_mass()))
    }
}
