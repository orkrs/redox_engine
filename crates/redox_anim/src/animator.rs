//! Clip playback state (an ECS component) and model container.

use crate::{AnimationClip, LocalPose, Skeleton, SkinnedGeometry};

/// Everything needed to animate and draw one rigged model.
#[derive(Clone, Debug, Default)]
pub struct SkinnedModel {
    pub skeleton: Skeleton,
    pub clips: Vec<AnimationClip>,
    pub geometry: SkinnedGeometry,
}

/// Plays one clip of a [`SkinnedModel`].
#[derive(Clone, Debug)]
pub struct Animator {
    pub clip: usize,
    pub time: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            clip: 0,
            time: 0.0,
            speed: 1.0,
            looping: true,
            playing: true,
        }
    }
}

impl Animator {
    /// Advances time; a non-looping clip stops at its end.
    pub fn advance(&mut self, dt: f32, clips: &[AnimationClip]) {
        if !self.playing {
            return;
        }
        let Some(clip) = clips.get(self.clip) else {
            return;
        };
        self.time += dt * self.speed;
        if self.looping && clip.duration > f32::EPSILON {
            self.time = self.time.rem_euclid(clip.duration);
        } else if self.time >= clip.duration {
            self.time = clip.duration;
            self.playing = false;
        }
    }

    /// Local pose at the current time (bind pose when the clip index is invalid).
    pub fn evaluate(&self, skeleton: &Skeleton, clips: &[AnimationClip]) -> LocalPose {
        match clips.get(self.clip) {
            Some(clip) => clip.sample(skeleton, self.time, self.looping),
            None => skeleton.bind_local_pose(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clips() -> Vec<AnimationClip> {
        vec![AnimationClip {
            name: "c".into(),
            duration: 2.0,
            ..Default::default()
        }]
    }

    #[test]
    fn looping_wraps() {
        let mut a = Animator::default();
        a.advance(2.5, &clips());
        assert!((a.time - 0.5).abs() < 1.0e-5);
        assert!(a.playing);
    }

    #[test]
    fn one_shot_stops_at_end() {
        let mut a = Animator {
            looping: false,
            ..Default::default()
        };
        a.advance(5.0, &clips());
        assert_eq!(a.time, 2.0);
        assert!(!a.playing);
    }

    #[test]
    fn speed_scales_time() {
        let mut a = Animator {
            speed: 2.0,
            ..Default::default()
        };
        a.advance(0.25, &clips());
        assert!((a.time - 0.5).abs() < 1.0e-5);
    }

    #[test]
    fn invalid_clip_index_is_bind_pose() {
        let a = Animator {
            clip: 7,
            ..Default::default()
        };
        let skeleton = Skeleton::default();
        assert!(a.evaluate(&skeleton, &clips()).bones.is_empty());
    }
}
