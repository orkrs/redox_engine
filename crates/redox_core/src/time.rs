//! Time management for the engine.

/// Tracks delta time and orchestrates fixed-timestep updates.
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
}

impl Time {
    /// Creates a new Time resource with the specified fixed delta time.
    ///
    /// Default `fixed_delta_time` is typically `1.0 / 60.0`.
    pub fn new(fixed_delta_time: f32) -> Self {
        Self {
            delta_time: 0.0,
            fixed_delta_time,
            time_scale: 1.0,
            total_time: 0.0,
            accumulator: 0.0,
        }
    }

    /// Advances the time resource by the measured frame delta.
    pub fn tick(&mut self, delta: f32) {
        let scaled_delta = delta * self.time_scale;
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
}
