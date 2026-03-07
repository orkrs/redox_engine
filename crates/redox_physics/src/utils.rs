use rapier3d::na::{Quaternion, UnitQuaternion, Vector3};
use redox_math::{Quat, Vec3};

/// Converts a `redox_math::Vec3` to `rapier3d` Vector3.
pub fn vec3_to_rapier(v: Vec3) -> Vector3<f32> {
    Vector3::new(v.x, v.y, v.z)
}

/// Converts a `rapier3d` Vector3 to `redox_math::Vec3`.
pub fn vec3_from_rapier(v: &Vector3<f32>) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// Converts a `redox_math::Quat` to `rapier3d` UnitQuaternion.
pub fn quat_to_rapier(q: Quat) -> UnitQuaternion<f32> {
    UnitQuaternion::from_quaternion(Quaternion::new(q.w, q.x, q.y, q.z))
}

/// Converts a `rapier3d` UnitQuaternion to `redox_math::Quat`.
pub fn quat_from_rapier(q: &UnitQuaternion<f32>) -> Quat {
    let q = q.quaternion();
    Quat::from_xyzw(q.i, q.j, q.k, q.w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vec3_conversion() {
        let original = Vec3::new(1.0, 2.0, 3.0);
        let rapier = vec3_to_rapier(original);
        let back = vec3_from_rapier(&rapier);
        assert_eq!(original, back);
    }
}
