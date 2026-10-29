//! Ragdolls: a template of bodies and joints that follows a skeleton, spawned as rigid bodies.
//!
//! A [`RagdollTemplate`] lists one body per bone. Each body is a collider centred `offset` away
//! from its bone, in the bone's local axes. Bones are assumed to point along their local +Y
//! towards their child, so capsules are laid out along Y.
//!
//! Joint axes and limits are given in the local frame of the child bone and applied to both
//! bodies, as `JointDesc` does. Bones whose local frames are rotated relative to each other need
//! a template written for those frames. The humanoid template assumes Mixamo-style rest poses.

use rapier3d::prelude::{ColliderBuilder, Pose, RigidBodyBuilder};
use redox_anim::Skeleton;
use redox_ecs::{Entity, World};
use redox_math::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::context::{PhysicsContext, PhysicsError};
use crate::joints::{JointDesc, JointKind};
use crate::shapes::CollisionLayers;

/// Length used for a bone without a child, in metres.
const LEAF_BONE_LENGTH: f32 = 0.15;
/// Capsule radius as a share of the bone length.
const RADIUS_PER_LENGTH: f32 = 0.12;
/// Smallest capsule radius in metres.
const MIN_RADIUS: f32 = 0.03;
/// Largest bend of knees and elbows in radians.
const HINGE_MAX: f32 = 2.4;
/// Twist allowed for most joints, in degrees.
const TWIST_DEG: f32 = 25.0;

/// Shape of one ragdoll body, in the body's local frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RagdollShape {
    /// Capsule along the local Y axis.
    Capsule {
        /// Radius in metres.
        radius: f32,
        /// Half of the cylindrical part's length in metres.
        half_height: f32,
    },
    /// Box with the given half extents.
    Box {
        /// Half extents along X, Y and Z in metres.
        half_extents: [f32; 3],
    },
    /// Sphere.
    Sphere {
        /// Radius in metres.
        radius: f32,
    },
}

/// Joint kind of a ragdoll bone, relative to its parent body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RagdollJointKind {
    /// Rotation about `axis` within `[min, max]` radians.
    Hinge {
        /// Hinge axis in the child bone's local frame.
        axis: [f32; 3],
        /// Smallest angle in radians.
        min: f32,
        /// Largest angle in radians.
        max: f32,
    },
    /// Rotation about the anchor with a swing cone and a twist range.
    Ball {
        /// Largest swing in radians on each of the two swing axes.
        swing: f32,
        /// Smallest twist in radians.
        twist_min: f32,
        /// Largest twist in radians.
        twist_max: f32,
    },
}

/// A joint between a bone's body and its parent body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RagdollJoint {
    /// The constraint.
    pub kind: RagdollJointKind,
    /// Joint point in the child bone's local frame.
    pub anchor: [f32; 3],
}

/// One body of a ragdoll template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RagdollBone {
    /// Name of the skeleton bone this body follows.
    pub bone: String,
    /// Index of the parent body in the template, `None` for the root.
    pub parent: Option<usize>,
    /// Collision shape.
    pub shape: RagdollShape,
    /// Mass in kilograms, must be positive.
    pub mass: f32,
    /// Joint to the parent body; `None` for the root.
    pub joint: Option<RagdollJoint>,
    /// Body centre relative to the bone, in the bone's local axes.
    pub offset: [f32; 3],
}

/// A set of ragdoll bodies with their joints, one body per listed bone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RagdollTemplate {
    /// Name of the template.
    pub name: String,
    /// Bodies, parents before children; the first has no parent.
    pub bones: Vec<RagdollBone>,
}

/// Errors of ragdoll templates and their spawning.
#[derive(Debug, thiserror::Error)]
pub enum RagdollError {
    /// The text is not a valid RON template.
    #[error("ragdoll template is not valid RON: {0}")]
    Parse(String),
    /// The template has no bones.
    #[error("ragdoll template has no bones")]
    Empty,
    /// A bone of the template or of the humanoid layout is missing from the skeleton.
    #[error("skeleton has no bone for `{0}`")]
    MissingBone(String),
    /// The template does not have exactly one root.
    #[error("ragdoll needs exactly one root bone, found {0}")]
    RootCount(usize),
    /// A bone's parent is not listed before it.
    #[error("bone `{0}` has a parent that is not listed before it")]
    ParentOrder(String),
    /// A bone's mass is not positive and finite.
    #[error("bone `{0}` needs a positive mass")]
    InvalidMass(String),
    /// The number of bone poses does not match the template.
    #[error("expected {expected} bone poses, got {got}")]
    PoseCount {
        /// Number of template bones.
        expected: usize,
        /// Number of poses given.
        got: usize,
    },
    /// The physics world refused a body or joint.
    #[error(transparent)]
    Physics(#[from] PhysicsError),
}

/// Handle of a spawned ragdoll: its bodies in template order and each body's offset.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollHandle {
    bodies: Vec<Entity>,
    offsets: Vec<Vec3>,
}

impl RagdollHandle {
    /// Entities of the bodies, in template order.
    pub fn bodies(&self) -> &[Entity] {
        &self.bodies
    }

    /// Offset of each body from its bone, in template order.
    pub fn offsets(&self) -> &[Vec3] {
        &self.offsets
    }
}

/// ECS component on a ragdoll body entity.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollBody {
    /// The ragdoll this body belongs to.
    pub ragdoll: RagdollHandle,
    /// Index of the body's bone in the template.
    pub bone_index: usize,
}

/// Bone matrix (model space) of a body with the given `offset`: the inverse of the body's offset.
pub fn bone_world_from_body(body_world: Mat4, offset: Vec3) -> Mat4 {
    body_world * Mat4::from_translation(-offset)
}

/// Which body of a humanoid a bone plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Part {
    Hips,
    Spine,
    Head,
    UpperArm,
    Forearm,
    Hand,
    Thigh,
    Calf,
    Foot,
}

/// Side of a limb; `Centre` for hips, spine and head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Side {
    Left,
    Right,
    Centre,
}

/// Joint preset of the humanoid layout, in degrees for the swing and twist.
#[derive(Clone, Copy)]
enum Preset {
    Root,
    Ball { swing_deg: f32, twist_deg: f32 },
    Hinge,
}

/// Humanoid layout: part, side, parent index into this table, joint preset. Parents come first.
const HUMANOID_LAYOUT: [(Part, Side, Option<usize>, Preset); 15] = [
    (Part::Hips, Side::Centre, None, Preset::Root),
    (Part::Spine, Side::Centre, Some(0), Preset::Ball { swing_deg: 20.0, twist_deg: TWIST_DEG }),
    (Part::Head, Side::Centre, Some(1), Preset::Ball { swing_deg: 35.0, twist_deg: TWIST_DEG }),
    (Part::UpperArm, Side::Left, Some(1), Preset::Ball { swing_deg: 60.0, twist_deg: TWIST_DEG }),
    (Part::Forearm, Side::Left, Some(3), Preset::Hinge),
    (Part::Hand, Side::Left, Some(4), Preset::Ball { swing_deg: 40.0, twist_deg: TWIST_DEG }),
    (Part::UpperArm, Side::Right, Some(1), Preset::Ball { swing_deg: 60.0, twist_deg: TWIST_DEG }),
    (Part::Forearm, Side::Right, Some(6), Preset::Hinge),
    (Part::Hand, Side::Right, Some(7), Preset::Ball { swing_deg: 40.0, twist_deg: TWIST_DEG }),
    (Part::Thigh, Side::Left, Some(0), Preset::Ball { swing_deg: 70.0, twist_deg: TWIST_DEG }),
    (Part::Calf, Side::Left, Some(9), Preset::Hinge),
    (Part::Foot, Side::Left, Some(10), Preset::Ball { swing_deg: 30.0, twist_deg: 15.0 }),
    (Part::Thigh, Side::Right, Some(0), Preset::Ball { swing_deg: 70.0, twist_deg: TWIST_DEG }),
    (Part::Calf, Side::Right, Some(12), Preset::Hinge),
    (Part::Foot, Side::Right, Some(13), Preset::Ball { swing_deg: 30.0, twist_deg: 15.0 }),
];

/// Mass in kilograms of each humanoid part (one body per side for limbs).
fn part_mass(part: Part) -> f32 {
    match part {
        Part::Hips => 12.0,
        Part::Spine => 15.0,
        Part::Head => 5.0,
        Part::UpperArm => 2.5,
        Part::Forearm => 1.5,
        Part::Hand => 0.6,
        Part::Thigh => 7.0,
        Part::Calf => 3.5,
        Part::Foot => 1.0,
    }
}

/// Splits a bone name into its side and the rest of the lowercase name.
fn strip_side(name: &str) -> (&str, Side) {
    for (word, side) in [("left", Side::Left), ("right", Side::Right)] {
        if let Some(rest) = name.strip_prefix(word).or_else(|| name.strip_suffix(word)) {
            return (rest, side);
        }
    }
    if let Some(rest) = name.strip_suffix('l') {
        return (rest, Side::Left);
    }
    if let Some(rest) = name.strip_suffix('r') {
        return (rest, Side::Right);
    }
    (name, Side::Centre)
}

/// The humanoid part a lowercase, alphanumeric name stands for.
fn part_of(name: &str) -> Option<Part> {
    if name.contains("hip") || name.contains("pelvis") {
        Some(Part::Hips)
    } else if name.contains("spine") {
        Some(Part::Spine)
    } else if name.contains("head") {
        Some(Part::Head)
    } else if name.contains("forearm") || name.contains("lowerarm") {
        Some(Part::Forearm)
    } else if name.ends_with("arm") {
        Some(Part::UpperArm)
    } else if name.contains("hand") {
        Some(Part::Hand)
    } else if name.contains("upleg") || name.contains("thigh") {
        Some(Part::Thigh)
    } else if name.contains("calf") || name.contains("leg") || name.contains("shin") {
        Some(Part::Calf)
    } else if name.contains("foot") {
        Some(Part::Foot)
    } else {
        None
    }
}

/// Recognises a skeleton bone name such as `mixamorig:LeftUpLeg`, `upperarm.L` or `calf_r`.
fn classify(name: &str) -> Option<(Part, Side)> {
    let folded: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let without_rig = folded.strip_prefix("mixamorig").unwrap_or(&folded);
    let digitless = without_rig.trim_end_matches(|c: char| c.is_ascii_digit());
    let (rest, side) = strip_side(digitless);
    part_of(rest).map(|part| (part, side))
}

/// Human-readable label of a humanoid part, used in error messages.
fn label(part: Part, side: Side) -> String {
    let side = match side {
        Side::Left => "left ",
        Side::Right => "right ",
        Side::Centre => "",
    };
    let part = match part {
        Part::Hips => "hips",
        Part::Spine => "spine",
        Part::Head => "head",
        Part::UpperArm => "upper arm",
        Part::Forearm => "forearm",
        Part::Hand => "hand",
        Part::Thigh => "thigh",
        Part::Calf => "calf",
        Part::Foot => "foot",
    };
    format!("{side}{part}")
}

impl RagdollTemplate {
    /// Parses a template from RON text.
    ///
    /// # Errors
    /// Returns [`RagdollError::Parse`] when the text is not a template in RON form.
    pub fn parse_ron(text: &str) -> Result<Self, RagdollError> {
        ron::from_str(text).map_err(|e| RagdollError::Parse(e.to_string()))
    }

    /// Checks that every bone exists in `skeleton` and that the structure is valid.
    ///
    /// # Errors
    /// Returns [`RagdollError::MissingBone`] for a bone the skeleton lacks, and the structural
    /// errors of [`Self::check_structure`].
    pub fn validate(&self, skeleton: &Skeleton) -> Result<(), RagdollError> {
        for bone in &self.bones {
            if skeleton.bone_index(&bone.bone).is_none() {
                return Err(RagdollError::MissingBone(bone.bone.clone()));
            }
        }
        self.check_structure()
    }

    /// Checks the template without a skeleton: bones exist, one root, parents first, masses positive.
    ///
    /// # Errors
    /// Returns the structural error found first.
    pub fn check_structure(&self) -> Result<(), RagdollError> {
        if self.bones.is_empty() {
            return Err(RagdollError::Empty);
        }
        let roots = self.bones.iter().filter(|b| b.parent.is_none()).count();
        if roots != 1 {
            return Err(RagdollError::RootCount(roots));
        }
        for (index, bone) in self.bones.iter().enumerate() {
            if bone.parent.is_some_and(|p| p >= index) {
                return Err(RagdollError::ParentOrder(bone.bone.clone()));
            }
            if !(bone.mass.is_finite() && bone.mass > 0.0) {
                return Err(RagdollError::InvalidMass(bone.bone.clone()));
            }
        }
        Ok(())
    }

    /// Builds the humanoid template for a skeleton, recognising bones by name.
    ///
    /// Needs hips, spine, head, and upper arm, forearm, hand, thigh, calf and foot on both sides.
    /// Capsule length is the distance to the bone's first child in the skeleton, or 0.15 m for
    /// a bone without one. Knees and elbows are hinges bending 0 to 2.4 rad; hips, spine, head
    /// and shoulders are ball joints.
    ///
    /// # Errors
    /// Returns [`RagdollError::MissingBone`] naming the first missing part.
    pub fn humanoid(skeleton: &Skeleton) -> Result<Self, RagdollError> {
        let classes: Vec<Option<(Part, Side)>> =
            skeleton.bones.iter().map(|b| classify(&b.name)).collect();
        let globals = skeleton.global_matrices(&skeleton.bind_local_pose());
        let mut bones = Vec::with_capacity(HUMANOID_LAYOUT.len());
        for (part, side, parent, preset) in HUMANOID_LAYOUT {
            let index = classes
                .iter()
                .position(|c| *c == Some((part, side)))
                .ok_or_else(|| RagdollError::MissingBone(label(part, side)))?;
            let length = bone_length(skeleton, &globals, index);
            let radius = (length * RADIUS_PER_LENGTH).max(MIN_RADIUS);
            bones.push(RagdollBone {
                bone: skeleton.bones[index].name.clone(),
                parent,
                shape: RagdollShape::Capsule {
                    radius,
                    half_height: (length * 0.5 - radius).max(0.0),
                },
                mass: part_mass(part),
                joint: humanoid_joint(preset),
                offset: [0.0, length * 0.5, 0.0],
            });
        }
        Ok(Self {
            name: "humanoid".to_string(),
            bones,
        })
    }

    /// Bone poses in template order, taken from `model` (model-space matrices in skeleton order).
    ///
    /// # Errors
    /// Returns [`RagdollError::MissingBone`] when a template bone is not in the skeleton, or
    /// [`RagdollError::PoseCount`] when `model` does not cover the skeleton.
    pub fn bone_poses(&self, skeleton: &Skeleton, model: &[Mat4]) -> Result<Vec<Mat4>, RagdollError> {
        if model.len() != skeleton.len() {
            return Err(RagdollError::PoseCount {
                expected: skeleton.len(),
                got: model.len(),
            });
        }
        self.bones
            .iter()
            .map(|bone| {
                skeleton
                    .bone_index(&bone.bone)
                    .map(|index| model[index])
                    .ok_or_else(|| RagdollError::MissingBone(bone.bone.clone()))
            })
            .collect()
    }
}

/// Length of the bone: the distance to its first child, or the leaf length.
fn bone_length(skeleton: &Skeleton, globals: &[Mat4], index: usize) -> f32 {
    let origin = globals[index].w_axis.truncate();
    skeleton
        .bones
        .iter()
        .position(|b| b.parent == Some(index))
        .map(|child| (globals[child].w_axis.truncate() - origin).length())
        .filter(|length| *length > 1e-4)
        .unwrap_or(LEAF_BONE_LENGTH)
}

/// The joint of a humanoid layout entry.
fn humanoid_joint(preset: Preset) -> Option<RagdollJoint> {
    let kind = match preset {
        Preset::Root => return None,
        Preset::Hinge => RagdollJointKind::Hinge {
            axis: [1.0, 0.0, 0.0],
            min: 0.0,
            max: HINGE_MAX,
        },
        Preset::Ball {
            swing_deg,
            twist_deg,
        } => RagdollJointKind::Ball {
            swing: swing_deg.to_radians(),
            twist_min: -twist_deg.to_radians(),
            twist_max: twist_deg.to_radians(),
        },
    };
    Some(RagdollJoint {
        kind,
        anchor: [0.0; 3],
    })
}

impl RagdollBone {
    /// Collider of the body, centred on the body's origin.
    fn collider(&self) -> ColliderBuilder {
        match self.shape {
            RagdollShape::Capsule {
                radius,
                half_height,
            } => ColliderBuilder::capsule_y(half_height, radius),
            RagdollShape::Box { half_extents: h } => ColliderBuilder::cuboid(h[0], h[1], h[2]),
            RagdollShape::Sphere { radius } => ColliderBuilder::ball(radius),
        }
    }
}

/// Physics joint description for a template joint whose anchors are already in body space.
fn joint_desc(joint: &RagdollJoint, anchor_a: Vec3, anchor_b: Vec3) -> JointDesc {
    let mut desc = match joint.kind {
        RagdollJointKind::Hinge { axis, min, max } => {
            let mut desc = JointDesc::new(JointKind::Hinge {
                axis: Vec3::from_array(axis),
            });
            desc.limits = Some((min, max));
            desc
        }
        RagdollJointKind::Ball {
            swing,
            twist_min,
            twist_max,
        } => JointDesc::new(JointKind::BallLimited {
            swing_limit: swing,
            twist_min,
            twist_max,
        }),
    };
    desc.anchor_a = anchor_a;
    desc.anchor_b = anchor_b;
    desc
}

impl PhysicsContext {
    /// Spawns the template as dynamic bodies at the given bone poses, joined by its joints.
    ///
    /// `bone_poses` are world matrices in template order. Every body starts with `root_velocity`.
    /// `group` is a collision membership mask: bodies of one ragdoll do not collide with each
    /// other, and collide with everything else. Joints disable contacts between joined bodies.
    ///
    /// # Errors
    /// Returns the structural error of the template, [`RagdollError::PoseCount`] for a pose count
    /// that does not match, or a physics error.
    pub fn spawn_ragdoll(
        &mut self,
        world: &mut World,
        template: &RagdollTemplate,
        bone_poses: &[Mat4],
        root_velocity: Vec3,
        group: u32,
    ) -> Result<RagdollHandle, RagdollError> {
        template.check_structure()?;
        if bone_poses.len() != template.bones.len() {
            return Err(RagdollError::PoseCount {
                expected: template.bones.len(),
                got: bone_poses.len(),
            });
        }
        let groups = CollisionLayers::new(group, !group).to_interaction_groups();
        let entities: Vec<Entity> = template.bones.iter().map(|_| world.spawn()).collect();
        let offsets: Vec<Vec3> = template
            .bones
            .iter()
            .map(|b| Vec3::from_array(b.offset))
            .collect();
        let body_poses: Vec<Mat4> = bone_poses
            .iter()
            .zip(&offsets)
            .map(|(pose, offset)| *pose * Mat4::from_translation(*offset))
            .collect();
        for (index, bone) in template.bones.iter().enumerate() {
            let (_, rotation, translation) = body_poses[index].to_scale_rotation_translation();
            let body = RigidBodyBuilder::dynamic()
                .pose(Pose::from_parts(translation, rotation))
                .linvel(root_velocity);
            let collider = bone.collider().mass(bone.mass).collision_groups(groups);
            self.add_rigid_body(entities[index], body, vec![collider])?;
        }
        for (index, bone) in template.bones.iter().enumerate() {
            let (Some(parent), Some(joint)) = (bone.parent, &bone.joint) else {
                continue;
            };
            let anchor_world = bone_poses[index].transform_point3(Vec3::from_array(joint.anchor));
            let desc = joint_desc(
                joint,
                body_poses[parent].inverse().transform_point3(anchor_world),
                body_poses[index].inverse().transform_point3(anchor_world),
            );
            self.add_joint(entities[parent], entities[index], desc)?;
        }
        let handle = RagdollHandle {
            bodies: entities.clone(),
            offsets,
        };
        for (index, entity) in entities.into_iter().enumerate() {
            world.add_component(
                entity,
                RagdollBody {
                    ragdoll: handle.clone(),
                    bone_index: index,
                },
            );
        }
        Ok(handle)
    }

    /// Removes the ragdoll's bodies and, with them, their joints. Bodies already gone are skipped.
    ///
    /// The ECS entities and their [`RagdollBody`] components stay in the world.
    pub fn remove_ragdoll(&mut self, ragdoll: &RagdollHandle) {
        for entity in ragdoll.bodies() {
            let _ = self.remove_rigid_body(*entity);
        }
    }
}
