//! Priority-based voice budget (plan item B8, remaining part).
//!
//! [`VoiceBudget`] limits how many sounds may play at once. When the budget is full, a new
//! voice may steal the slot of a strictly lower-priority voice; otherwise it is rejected.
//! Time is supplied by the caller, so the budget has no clock of its own.

/// A voice occupying one slot of the budget.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Voice {
    id: u64,
    priority: u8,
    start: f32,
    end: f32,
}

impl Voice {
    /// Creates a voice that starts at `start` and lasts `duration` seconds.
    fn new(id: u64, priority: u8, start: f32, duration: f32) -> Self {
        Self {
            id,
            priority,
            start,
            end: start + sanitize_duration(duration),
        }
    }
}

/// What the budget decided for a voice request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceDecision {
    /// A free slot was available; the voice plays.
    Play,
    /// The voice took the slot of the voice with this id, which is stopped.
    Steal(u64),
    /// The budget is full of equal or higher priority voices; nothing changed.
    Reject,
}

/// Limits the number of simultaneous voices, with priority-based stealing.
#[derive(Clone, Debug)]
pub struct VoiceBudget {
    max_voices: usize,
    voices: Vec<Voice>,
}

impl VoiceBudget {
    /// Creates a budget with `max_voices` slots. Zero slots reject every request.
    pub fn new(max_voices: usize) -> Self {
        Self {
            max_voices,
            voices: Vec::new(),
        }
    }

    /// Requests a slot for a voice with `id` and `priority` (higher wins) starting at `now`
    /// and lasting `duration` seconds.
    ///
    /// A free slot is taken first. If the budget is full, the lowest-priority voice (the oldest
    /// among equals) is stolen, but only when it has a strictly lower priority than the request.
    pub fn request(&mut self, id: u64, priority: u8, now: f32, duration: f32) -> VoiceDecision {
        self.expire(now);
        if self.voices.len() < self.max_voices {
            self.voices.push(Voice::new(id, priority, now, duration));
            return VoiceDecision::Play;
        }
        let Some(weakest) = self.weakest() else {
            return VoiceDecision::Reject;
        };
        let Some(slot) = self.voices.get_mut(weakest) else {
            return VoiceDecision::Reject;
        };
        if slot.priority >= priority {
            return VoiceDecision::Reject;
        }
        let stolen = slot.id;
        self.voices.remove(weakest);
        self.voices.push(Voice::new(id, priority, now, duration));
        VoiceDecision::Steal(stolen)
    }

    /// Number of voices still playing at time `now`.
    pub fn active(&self, now: f32) -> usize {
        self.voices.iter().filter(|voice| voice.end > now).count()
    }

    /// Frees the slot of voice `id` before its time is up. False when no such voice is held.
    pub fn release(&mut self, id: u64) -> bool {
        match self.voices.iter().position(|voice| voice.id == id) {
            Some(index) => {
                self.voices.remove(index);
                true
            }
            None => false,
        }
    }

    /// Ids of the voices still playing at `now`, oldest first (by start time; ties by insertion
    /// order).
    pub fn ids(&self, now: f32) -> Vec<u64> {
        let mut live: Vec<&Voice> = self.voices.iter().filter(|v| v.end > now).collect();
        live.sort_by(|a, b| a.start.total_cmp(&b.start));
        live.into_iter().map(|voice| voice.id).collect()
    }

    /// Sets the limit to `max_voices` and returns the ids evicted to fit it, lowest priority first
    /// and the oldest first among equals. Voices that already ended by `now` are dropped silently
    /// and not reported.
    pub fn set_max_voices(&mut self, max_voices: usize, now: f32) -> Vec<u64> {
        self.expire(now);
        self.max_voices = max_voices;
        let mut evicted = Vec::new();
        while self.voices.len() > self.max_voices {
            let Some(weakest) = self.weakest() else {
                break;
            };
            evicted.push(self.voices.remove(weakest).id);
        }
        evicted
    }

    /// Drops voices that have ended by `now`.
    fn expire(&mut self, now: f32) {
        self.voices.retain(|voice| voice.end > now);
    }

    /// Index of the lowest-priority voice, the oldest one among equals.
    fn weakest(&self) -> Option<usize> {
        self.voices
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                a.priority
                    .cmp(&b.priority)
                    .then(a.start.total_cmp(&b.start))
            })
            .map(|(index, _)| index)
    }
}

/// Treats negative and NaN durations as zero, so such voices end immediately.
fn sanitize_duration(duration: f32) -> f32 {
    if duration > 0.0 { duration } else { 0.0 }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_length_voice_never_occupies_a_slot() {
        let mut budget = VoiceBudget::new(1);
        assert_eq!(budget.request(1, 5, 0.0, 0.0), VoiceDecision::Play);
        assert_eq!(budget.active(0.0), 0);
        assert_eq!(budget.request(2, 5, 0.0, 1.0), VoiceDecision::Play);
    }

    #[test]
    fn negative_and_nan_durations_end_immediately() {
        let mut budget = VoiceBudget::new(2);
        budget.request(1, 5, 0.0, -3.0);
        budget.request(2, 5, 0.0, f32::NAN);
        assert_eq!(budget.active(0.0), 0);
    }

    #[test]
    fn the_oldest_of_equal_lowest_priority_is_stolen_even_if_added_out_of_order() {
        let mut budget = VoiceBudget::new(2);
        budget.request(1, 4, 2.0, 10.0);
        budget.request(2, 4, 1.0, 10.0);
        assert_eq!(budget.request(3, 9, 3.0, 1.0), VoiceDecision::Steal(2));
    }

    #[test]
    fn a_zero_budget_rejects_even_the_highest_priority() {
        let mut budget = VoiceBudget::new(0);
        assert_eq!(
            budget.request(1, u8::MAX, 0.0, f32::INFINITY),
            VoiceDecision::Reject
        );
        assert_eq!(budget.active(0.0), 0);
    }

    #[test]
    fn an_infinite_duration_voice_keeps_its_slot() {
        let mut budget = VoiceBudget::new(1);
        budget.request(1, 1, 0.0, f32::INFINITY);
        assert_eq!(budget.request(2, 1, 1e9, 1.0), VoiceDecision::Reject);
    }
}
