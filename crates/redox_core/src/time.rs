//! Time management for the engine.

/// Tracks delta time and orchestrates fixed-timestep updates.
///
/// Stored as a [`World`](redox_ecs::World) resource; systems read it via
/// `world.get_resource::<Time>()`.
#[derive(Debug, Clone)]
pub struct Time {
    /// The time in seconds that passed during the last frame.
    pub delta_time: f32,
    /// The fixed timestep interval in seconds used for physics and fixed updates.
    pub fixed_delta_time: f32,
    /// Multiplier for `delta_time`, allowing for slow motion or fast forwarding.
    pub time_scale: f32,
    /// The total elapsed time in seconds since the engine started.
    pub total_time: f32,
    /// Accumulator for fractional fixed timesteps.
    pub accumulator: f32,
    /// Alpha interpolation factor [0.0, 1.0] for rendering interpolation between physics ticks.
    pub alpha: f32,
}

/// Upper bound for a single frame delta (seconds). Prevents the "tab was in
/// the background for a minute" explosion from teleporting physics.
pub const MAX_DELTA: f32 = 0.25;

impl Time {
    /// Creates a new Time resource with the specified fixed delta time.
    pub fn new(fixed_delta_time: f32) -> Self {
        Self {
            delta_time: 0.0,
            fixed_delta_time,
            time_scale: 1.0,
            total_time: 0.0,
            accumulator: 0.0,
            alpha: 1.0,
        }
    }

    /// Advances the time resource by the measured frame delta.
    ///
    /// The delta is clamped to [`MAX_DELTA`] so a single long stall cannot
    /// queue unbounded physics work.
    pub fn tick(&mut self, delta: f32) {
        let scaled_delta = (delta * self.time_scale).min(MAX_DELTA);
        self.delta_time = scaled_delta;
        self.total_time += scaled_delta;
        self.accumulator += scaled_delta;
    }

    /// Checks if a fixed update should occur.
    pub fn should_step_fixed(&self) -> bool {
        self.accumulator >= self.fixed_delta_time
    }

    /// Consumes one fixed step interval from the accumulator.
    pub fn consume_fixed_step(&mut self) {
        self.accumulator -= self.fixed_delta_time;
    }

    /// Discards all accumulated fixed steps (spiral-of-death escape hatch).
    ///
    /// Called by the dispatcher when the per-frame step cap is exhausted so
    /// debt does not accumulate across frames.
    pub fn reset_accumulator(&mut self) {
        self.accumulator = 0.0;
        self.alpha = 0.0;
    }

    /// Updates `alpha` interpolation factor based on remaining accumulator.
    pub fn update_alpha(&mut self) {
        self.alpha = if self.fixed_delta_time > 0.0 {
            (self.accumulator / self.fixed_delta_time).clamp(0.0, 1.0)
        } else {
            1.0
        };
    }
}
