//! Music crossfade state machine (plan item B8, remaining part).
//!
//! [`MusicDirector`] fades one track in while every other track fades out. Each track has a
//! linear level in `[0, 1]`; the reported gain is `sin(level * PI / 2)`, so a symmetrical
//! crossfade keeps equal power. The director has no audio device: the caller applies the
//! gains from each [`MusicFrame`] to its own players.

use std::f32::consts::FRAC_PI_2;

/// The fade state of one track.
#[derive(Clone, Debug, PartialEq)]
struct Fade {
    name: String,
    level: f32,
    target: f32,
}

/// The result of one [`MusicDirector::update`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicFrame {
    /// Every audible track with its gain, in the order the tracks were started.
    pub gains: Vec<(String, f32)>,
    /// Tracks that reached silence during this update and were removed. Reported once.
    pub finished: Vec<String>,
}

/// Crossfades between music tracks with equal-power gains.
#[derive(Clone, Debug)]
pub struct MusicDirector {
    crossfade_seconds: f32,
    current: Option<String>,
    fades: Vec<Fade>,
}

impl MusicDirector {
    /// Creates a director. A `crossfade_seconds` of zero or less (or NaN) switches instantly.
    pub fn new(crossfade_seconds: f32) -> Self {
        Self {
            crossfade_seconds,
            current: None,
            fades: Vec::new(),
        }
    }

    /// Starts `track` fading in while every other track fades out.
    ///
    /// Playing the track that is already current does nothing. A track that is still fading
    /// out turns around from its current level.
    pub fn play(&mut self, track: &str) {
        if self.current.as_deref() == Some(track) {
            return;
        }
        self.current = Some(track.to_owned());
        if !self.fades.iter().any(|fade| fade.name == track) {
            self.fades.push(Fade {
                name: track.to_owned(),
                level: 0.0,
                target: 0.0,
            });
        }
        self.retarget();
    }

    /// Fades every track out. [`current`](Self::current) is `None` immediately.
    pub fn stop(&mut self) {
        self.current = None;
        self.retarget();
    }

    /// The track that is currently selected to play, if any.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// True when no track is audible any more.
    pub fn is_idle(&self) -> bool {
        self.fades.is_empty()
    }

    /// Advances all fades by `dt` seconds.
    ///
    /// A negative or NaN `dt` changes nothing. The returned frame lists the audible tracks and
    /// the tracks that went silent in this update.
    pub fn update(&mut self, dt: f32) -> MusicFrame {
        if dt.is_nan() || dt < 0.0 {
            return MusicFrame {
                gains: self.gains(),
                finished: Vec::new(),
            };
        }
        let step = self.step_for(dt);
        for fade in &mut self.fades {
            fade.level = move_towards(fade.level, fade.target, step);
        }
        let mut finished = Vec::new();
        self.fades.retain(|fade| {
            let silent = fade.target == 0.0 && fade.level == 0.0;
            if silent {
                finished.push(fade.name.clone());
            }
            !silent
        });
        MusicFrame {
            gains: self.gains(),
            finished,
        }
    }

    /// Points every fade at the current track (target 1) or at silence (target 0).
    fn retarget(&mut self) {
        let current = self.current.as_deref();
        for fade in &mut self.fades {
            fade.target = if Some(fade.name.as_str()) == current {
                1.0
            } else {
                0.0
            };
        }
    }

    /// The level change allowed for a time step. Instant mode moves every level in one step.
    fn step_for(&self, dt: f32) -> f32 {
        if self.crossfade_seconds > 0.0 {
            dt / self.crossfade_seconds
        } else {
            1.0
        }
    }

    /// Equal-power gains for every track that is still in the list.
    fn gains(&self) -> Vec<(String, f32)> {
        self.fades
            .iter()
            .map(|fade| (fade.name.clone(), equal_power_gain(fade.level)))
            .collect()
    }
}

/// Moves `level` towards `target` by at most `step`, never overshooting.
fn move_towards(level: f32, target: f32, step: f32) -> f32 {
    if level < target {
        (level + step).min(target)
    } else {
        (level - step).max(target)
    }
}

/// Maps a linear fade level in `[0, 1]` to an equal-power gain.
fn equal_power_gain(level: f32) -> f32 {
    (level * FRAC_PI_2).sin()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn gain_of(frame: &MusicFrame, track: &str) -> Option<f32> {
        frame
            .gains
            .iter()
            .find(|(t, _)| t == track)
            .map(|(_, g)| *g)
    }

    #[test]
    fn a_nan_crossfade_switches_instantly() {
        let mut music = MusicDirector::new(f32::NAN);
        music.play("a");
        music.update(0.0);
        music.play("b");
        let frame = music.update(0.0);
        assert_eq!(frame.finished, vec!["a"]);
        assert_eq!(gain_of(&frame, "b"), Some(1.0));
    }

    #[test]
    fn a_new_director_is_idle_with_no_current_track() {
        let music = MusicDirector::new(1.0);
        assert!(music.is_idle());
        assert_eq!(music.current(), None);
    }

    #[test]
    fn stop_on_an_idle_director_stays_idle() {
        let mut music = MusicDirector::new(1.0);
        music.stop();
        assert!(music.is_idle());
        assert!(music.update(1.0).finished.is_empty());
    }

    #[test]
    fn stopping_a_track_that_just_started_retires_it_without_audible_gain() {
        let mut music = MusicDirector::new(1.0);
        music.play("a");
        music.stop();
        let frame = music.update(0.5);
        assert_eq!(frame.finished, vec!["a"]);
        assert!(frame.gains.is_empty());
    }

    #[test]
    fn an_infinite_time_step_lands_on_the_target_without_panicking() {
        let mut music = MusicDirector::new(1.0);
        music.play("a");
        let frame = music.update(f32::INFINITY);
        assert_eq!(gain_of(&frame, "a"), Some(1.0));
    }
}
