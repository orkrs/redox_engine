//! Data-driven sound cues (plan item B8, first part).
//!
//! A [`SoundCue`] names a set of sound assets (its variants), a volume and pitch range,
//! a cooldown and the mixer bus it plays on. Cues are authored as RON and loaded with
//! [`CueLibrary::parse`]. At runtime [`CueSelector`] decides which variant plays and how
//! loud and fast it is, and [`AudioContext::play_cue`] plays the result.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::context::AudioContext;
use crate::mixer::{MixerBusId, PlayRequest};
use crate::voices::VoiceDecision;

/// Seed used when the system clock is unavailable or before the Unix epoch.
const FALLBACK_SEED: u64 = 0x5EED_C0DE;

/// A named sound effect with variation: which sounds to pick from and how to vary them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SoundCue {
    /// Unique cue name used to trigger it from gameplay code.
    pub name: String,
    /// Sound asset names to choose from, e.g. `"step1"`.
    pub variants: Vec<String>,
    /// Linear gain range `(min, max)`, picked uniformly.
    pub volume: (f32, f32),
    /// Playback-rate range `(min, max)`, picked uniformly. `1.0` is the original pitch.
    pub pitch: (f32, f32),
    /// Seconds that must pass after a play before the cue can play again.
    pub cooldown: f32,
    /// Mixer bus the cue is routed to.
    pub bus: MixerBusId,
    /// Relative importance of the cue. Higher wins when the cue voice budget is full.
    pub priority: u8,
}

/// The outcome of selecting a cue: which variant to play and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CuePlay {
    /// Index into [`SoundCue::variants`].
    pub variant: usize,
    /// Linear gain to play at.
    pub volume: f32,
    /// Playback rate to play at (`1.0` is the original pitch).
    pub pitch: f32,
}

/// A source of uniform random numbers for cue selection.
pub trait CueRng {
    /// Returns a uniform value in `[0, 1)`.
    fn next_f32(&mut self) -> f32;
}

/// The SplitMix64 pseudo-random generator. Deterministic for a given seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    /// Advances the state and returns the next 64-bit output.
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl CueRng for SplitMix64 {
    fn next_f32(&mut self) -> f32 {
        // The upper 24 bits fit exactly in an f32 mantissa, so the result is in [0, 1).
        const SCALE: f32 = 1.0 / 16_777_216.0;
        (self.next_u64() >> 40) as f32 * SCALE
    }
}

/// Last play of a cue, kept per cue name by [`CueSelector`].
#[derive(Clone, Copy, Debug)]
struct CueHistory {
    last_variant: usize,
    last_play: f32,
}

/// Chooses variants, volumes and pitches for cues, and enforces cooldowns.
///
/// Remembers the last variant and play time of each cue name, so the same variant is not
/// repeated back to back and cooldowns apply per cue.
#[derive(Debug)]
pub struct CueSelector {
    history: HashMap<String, CueHistory>,
    rng: SplitMix64,
}

impl CueSelector {
    /// Creates a selector whose internal random generator is seeded from the clock.
    pub fn new() -> Self {
        Self {
            history: HashMap::new(),
            rng: SplitMix64(clock_seed()),
        }
    }

    /// Selects what to play for `cue` at time `now` (seconds), drawing randomness from `rng`.
    ///
    /// Returns `None` when the cue has no variants or is still cooling down. A rejected call
    /// changes no state. `now` may go backwards (for example after a clock reset); the cue is
    /// then treated as ready.
    pub fn select(&mut self, cue: &SoundCue, now: f32, rng: &mut dyn CueRng) -> Option<CuePlay> {
        select_with(&mut self.history, cue, now, rng)
    }

    /// Selects using the selector's own generator. Used by [`AudioContext::play_cue`].
    fn select_own_rng(&mut self, cue: &SoundCue, now: f32) -> Option<CuePlay> {
        select_with(&mut self.history, cue, now, &mut self.rng)
    }
}

impl Default for CueSelector {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared selection logic over a history map and a random source.
fn select_with(
    history: &mut HashMap<String, CueHistory>,
    cue: &SoundCue,
    now: f32,
    rng: &mut dyn CueRng,
) -> Option<CuePlay> {
    let variant_count = cue.variants.len();
    if variant_count == 0 {
        return None;
    }
    let previous = history.get(&cue.name).copied();
    if previous.is_some_and(|p| is_cooling_down(p.last_play, now, cue.cooldown)) {
        return None;
    }
    let avoid = previous.map(|p| p.last_variant);
    let variant = pick_variant(variant_count, avoid, rng);
    let play = CuePlay {
        variant,
        volume: pick_in_range(cue.volume, rng),
        pitch: pick_in_range(cue.pitch, rng),
    };
    history.insert(
        cue.name.clone(),
        CueHistory {
            last_variant: variant,
            last_play: now,
        },
    );
    Some(play)
}

/// True while less than `cooldown` seconds have passed since `last_play`.
///
/// A negative elapsed time (the clock went backwards) is never cooling down.
fn is_cooling_down(last_play: f32, now: f32, cooldown: f32) -> bool {
    let elapsed = now - last_play;
    elapsed >= 0.0 && elapsed < cooldown
}

/// Picks a variant index uniformly, excluding `avoid` when there are two or more variants.
///
/// `count` must be at least 1.
fn pick_variant(count: usize, avoid: Option<usize>, rng: &mut dyn CueRng) -> usize {
    let avoid = avoid.filter(|&a| a < count);
    match avoid {
        Some(avoid) if count >= 2 => {
            let slot = uniform_index(count - 1, rng);
            if slot >= avoid { slot + 1 } else { slot }
        }
        _ => uniform_index(count, rng),
    }
}

/// Draws an index in `0..count` (`count` must be at least 1).
fn uniform_index(count: usize, rng: &mut dyn CueRng) -> usize {
    let index = (rng.next_f32() * count as f32) as usize;
    index.min(count - 1)
}

/// Draws a value uniformly from `range`. A degenerate range returns its single value exactly.
fn pick_in_range((min, max): (f32, f32), rng: &mut dyn CueRng) -> f32 {
    min + (max - min) * rng.next_f32()
}

/// Seed derived from the current time, falling back to a constant.
fn clock_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_nanos()).ok())
        .unwrap_or(FALLBACK_SEED)
}

/// Errors from loading a cue library.
#[derive(Debug, Error)]
pub enum CueError {
    /// The text is not a valid RON list of cues.
    #[error("cue file could not be parsed: {0}")]
    Parse(String),
    /// Two cues share the same name.
    #[error("duplicate cue name `{0}`")]
    DuplicateName(String),
    /// A cue has a bad value (empty name, reversed range, negative or NaN number).
    #[error("invalid cue: {0}")]
    Invalid(String),
}

/// A set of sound cues loaded from RON, looked up by name.
#[derive(Clone, Debug, Default)]
pub struct CueLibrary {
    /// Cues in the order they were defined.
    cues: Vec<SoundCue>,
    /// Position of each cue in `cues`, keyed by name.
    index: HashMap<String, usize>,
}

impl CueLibrary {
    /// Parses a RON list of [`SoundCue`] and validates every cue.
    ///
    /// An empty list is valid. Fails with [`CueError::Parse`] on bad RON,
    /// [`CueError::DuplicateName`] on a repeated name and [`CueError::Invalid`] on a bad value.
    pub fn parse(ron_text: &str) -> Result<Self, CueError> {
        let cues: Vec<SoundCue> =
            ron::from_str(ron_text).map_err(|e| CueError::Parse(e.to_string()))?;
        let mut library = Self::default();
        for cue in cues {
            library.insert(cue)?;
        }
        Ok(library)
    }

    /// Validates and adds one cue, rejecting duplicate names.
    fn insert(&mut self, cue: SoundCue) -> Result<(), CueError> {
        validate(&cue)?;
        if self.index.contains_key(&cue.name) {
            return Err(CueError::DuplicateName(cue.name));
        }
        self.index.insert(cue.name.clone(), self.cues.len());
        self.cues.push(cue);
        Ok(())
    }

    /// Looks up a cue by name.
    pub fn get(&self, name: &str) -> Option<&SoundCue> {
        self.index.get(name).and_then(|&i| self.cues.get(i))
    }

    /// Number of cues in the library.
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    /// True when the library holds no cues.
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }
}

/// Checks one cue's values. Returns [`CueError::Invalid`] describing the first problem.
fn validate(cue: &SoundCue) -> Result<(), CueError> {
    if cue.name.is_empty() {
        return Err(CueError::Invalid("cue name is empty".into()));
    }
    check_range(&cue.name, "volume", cue.volume)?;
    check_range(&cue.name, "pitch", cue.pitch)?;
    if cue.cooldown.is_nan() || cue.cooldown < 0.0 {
        return Err(CueError::Invalid(format!(
            "cue `{}`: cooldown must be a non-negative number",
            cue.name
        )));
    }
    Ok(())
}

/// Checks that a `(min, max)` range is not NaN and has `min <= max`.
fn check_range(cue_name: &str, field: &str, (min, max): (f32, f32)) -> Result<(), CueError> {
    if min.is_nan() || max.is_nan() {
        return Err(CueError::Invalid(format!(
            "cue `{cue_name}`: {field} is NaN"
        )));
    }
    if min > max {
        return Err(CueError::Invalid(format!(
            "cue `{cue_name}`: {field} range is reversed ({min} > {max})"
        )));
    }
    Ok(())
}

impl AudioContext {
    /// Plays the next variant of cue `name` on its bus, if the selector and the voice budget
    /// allow it.
    ///
    /// Returns `None` for an unknown name, when the selector rejects the cue (no variants or
    /// cooling down), or when the voice budget rejects it (full of equal or higher priority
    /// voices). Otherwise returns the chosen [`CuePlay`]. The sound is played through the mixer
    /// when a device and the sound asset are available; without them the call still returns
    /// the choice and does nothing else.
    pub fn play_cue(
        &mut self,
        library: &CueLibrary,
        selector: &mut CueSelector,
        name: &str,
        now: f32,
    ) -> Option<CuePlay> {
        let cue = library.get(name)?;
        let play = selector.select_own_rng(cue, now)?;
        // A stolen voice's audio is not stopped: the context keeps no handle for cue voices.
        if self.request_cue_voice(cue.priority, now) == VoiceDecision::Reject {
            return None;
        }
        if let Some(sound) = cue.variants.get(play.variant) {
            let request = PlayRequest::one_shot(cue.bus, play.volume);
            self.play_named_pitched(sound, request, f64::from(play.pitch));
        }
        Some(play)
    }
}
