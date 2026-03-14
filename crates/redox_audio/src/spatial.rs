//! Spatial audio helpers.
use crate::components::{AudioEmitter, AudioListener};

pub fn distance_attenuation(emitter: &AudioEmitter, listener: &AudioListener) -> f32 {
    let dist = emitter.position.distance(listener.position);
    if dist >= emitter.max_distance {
        return 0.0;
    }
    1.0 - (dist / emitter.max_distance)
}

pub fn stereo_pan(emitter: &AudioEmitter, listener: &AudioListener) -> f32 {
    let vec_to_emitter = emitter.position - listener.position;
    if vec_to_emitter.length_squared() < 0.001 {
        return 0.0;
    }
    let to_emitter = vec_to_emitter.normalize();
    let right = listener.forward.cross(listener.up).normalize(); // Assuming right-handed
    to_emitter.dot(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;

    #[test]
    fn attenuation_at_zero_distance() {
        let e = AudioEmitter::new("a.wav", Vec3::ZERO);
        let l = AudioListener::new(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let atten = distance_attenuation(&e, &l);
        assert!(
            (atten - 1.0).abs() < 0.001,
            "At same position, attenuation should be 1.0"
        );
    }
    // ... rest of tests

    #[test]
    fn attenuation_at_max_distance() {
        let mut e = AudioEmitter::new("a.wav", Vec3::new(50.0, 0.0, 0.0));
        e.max_distance = 50.0;
        let l = AudioListener::new(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let atten = distance_attenuation(&e, &l);
        assert!(
            atten.abs() < 0.001,
            "At max distance, attenuation should be 0.0"
        );
    }

    #[test]
    fn pan_right() {
        let e = AudioEmitter::new("a.wav", Vec3::new(10.0, 0.0, 0.0));
        let l = AudioListener::new(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let pan = stereo_pan(&e, &l);
        // Listener facing -Z, right is +X → emitter on +X → pan ≈ +1.0
        assert!(
            pan > 0.5,
            "Emitter on the right should have positive pan, got {}",
            pan
        );
    }

    #[test]
    fn pan_left() {
        let e = AudioEmitter::new("a.wav", Vec3::new(-10.0, 0.0, 0.0));
        let l = AudioListener::new(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let pan = stereo_pan(&e, &l);
        assert!(
            pan < -0.5,
            "Emitter on the left should have negative pan, got {}",
            pan
        );
    }
}
