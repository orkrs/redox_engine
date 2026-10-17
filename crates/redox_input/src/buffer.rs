//! Input feel helpers: input buffering and coyote time.

/// A timer that tracks grace period after a condition becomes false (e.g., leaving a ledge).
#[derive(Debug, Clone, Copy)]
pub struct CoyoteTimer {
    duration: f32,
    timer: f32,
}

impl CoyoteTimer {
    /// Creates a timer with a grace period of `duration_seconds`.
    pub fn new(duration_seconds: f32) -> Self {
        Self {
            duration: duration_seconds,
            timer: 0.0,
        }
    }

    /// Resets the coyote window to full duration (e.g., when standing on ground).
    pub fn reset(&mut self) {
        self.timer = self.duration;
    }

    /// Advances the timer by delta time.
    pub fn tick(&mut self, dt: f32) {
        if self.timer > 0.0 {
            self.timer = (self.timer - dt).max(0.0);
        }
    }

    /// Returns `true` if within the coyote window.
    pub fn is_available(&self) -> bool {
        self.timer > 0.0
    }

    /// Consumes the coyote window, returning true if it was available.
    pub fn consume(&mut self) -> bool {
        if self.is_available() {
            self.timer = 0.0;
            true
        } else {
            false
        }
    }
}

/// A buffer that holds an input intent for a short duration (e.g., jump buffer).
#[derive(Debug, Clone, Copy)]
pub struct InputBuffer {
    buffer_window: f32,
    time_remaining: f32,
}

impl InputBuffer {
    /// Creates a buffer that remembers a press for `window_seconds`.
    pub fn new(window_seconds: f32) -> Self {
        Self {
            buffer_window: window_seconds,
            time_remaining: 0.0,
        }
    }

    /// Registers that the input action was pressed.
    pub fn trigger(&mut self) {
        self.time_remaining = self.buffer_window;
    }

    /// Advances the buffer timer by delta time.
    pub fn tick(&mut self, dt: f32) {
        if self.time_remaining > 0.0 {
            self.time_remaining = (self.time_remaining - dt).max(0.0);
        }
    }

    /// Returns true if an input is buffered and unconsumed.
    pub fn is_buffered(&self) -> bool {
        self.time_remaining > 0.0
    }

    /// Consumes the buffered input if present, returning `true`.
    pub fn consume(&mut self) -> bool {
        if self.is_buffered() {
            self.time_remaining = 0.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coyote_timer() {
        let mut coyote = CoyoteTimer::new(0.15);
        assert!(!coyote.is_available());
        coyote.reset();
        assert!(coyote.is_available());
        coyote.tick(0.05);
        assert!(coyote.is_available());
        assert!(coyote.consume());
        assert!(!coyote.is_available());
        assert!(!coyote.consume());

        coyote.reset();
        coyote.tick(0.20);
        assert!(!coyote.is_available());
    }

    #[test]
    fn test_input_buffer() {
        let mut buffer = InputBuffer::new(0.1);
        assert!(!buffer.is_buffered());
        buffer.trigger();
        assert!(buffer.is_buffered());
        buffer.tick(0.05);
        assert!(buffer.is_buffered());
        assert!(buffer.consume());
        assert!(!buffer.is_buffered());

        buffer.trigger();
        buffer.tick(0.15);
        assert!(!buffer.is_buffered());
    }
}
