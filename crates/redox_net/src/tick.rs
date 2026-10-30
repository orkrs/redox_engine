//! Fixed server tick and the client-side estimate of the current server tick.

use std::time::Duration;

/// Authoritative simulation rate of the server.
pub const SERVER_TICK_RATE_HZ: u32 = 30;
/// Length of one server tick.
pub const SERVER_TICK_DURATION: Duration =
    Duration::from_nanos(1_000_000_000 / SERVER_TICK_RATE_HZ as u64);
/// Upper bound on ticks run by one poll, so a long stall does not cause a catch-up spiral.
pub const MAX_TICKS_PER_POLL: u32 = 8;

/// Converts elapsed time into whole server ticks. The remainder stays in `accumulator`.
pub fn drain_fixed_ticks(accumulator: &mut Duration) -> u32 {
    let mut ticks = 0;
    while *accumulator >= SERVER_TICK_DURATION && ticks < MAX_TICKS_PER_POLL {
        *accumulator -= SERVER_TICK_DURATION;
        ticks += 1;
    }
    if *accumulator >= SERVER_TICK_DURATION {
        *accumulator = Duration::ZERO;
    }
    ticks
}

/// Weight of each new sample in the moving average.
const SMOOTHING: f64 = 0.1;

/// Estimates the current server tick from snapshots. Each snapshot carries the tick it was taken
/// at; half the round-trip time is added as the one-way delay, and the estimate runs forward with
/// local time between snapshots.
#[derive(Debug, Default, Clone, Copy)]
pub struct TickEstimator {
    estimate: Option<f64>,
}

impl TickEstimator {
    /// Moves the estimate forward by the local time `dt`.
    pub fn advance(&mut self, dt: Duration) {
        if let Some(value) = self.estimate.as_mut() {
            *value += dt.as_secs_f64() * f64::from(SERVER_TICK_RATE_HZ);
        }
    }

    /// Folds in a snapshot that was taken at `server_tick` and arrived after `rtt / 2`.
    pub fn observe(&mut self, server_tick: u64, rtt: Duration) {
        let one_way_ticks = rtt.as_secs_f64() * 0.5 * f64::from(SERVER_TICK_RATE_HZ);
        let sample = server_tick as f64 + one_way_ticks;
        self.estimate = Some(match self.estimate {
            None => sample,
            Some(value) => value + SMOOTHING * (sample - value),
        });
    }

    /// Whole server tick, or `None` before the first snapshot.
    pub fn estimate(&self) -> Option<u64> {
        self.estimate.map(|value| value.max(0.0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SplitMix64;

    #[test]
    fn whole_ticks_are_drained_and_the_remainder_is_kept() {
        let mut accumulator = SERVER_TICK_DURATION * 3 + Duration::from_millis(1);
        assert_eq!(drain_fixed_ticks(&mut accumulator), 3);
        assert_eq!(accumulator, Duration::from_millis(1));
    }

    #[test]
    fn a_long_stall_runs_at_most_the_tick_budget() {
        let mut accumulator = Duration::from_secs(10);
        assert_eq!(drain_fixed_ticks(&mut accumulator), MAX_TICKS_PER_POLL);
        assert_eq!(accumulator, Duration::ZERO);
    }

    #[test]
    fn the_server_tick_estimate_converges_to_the_true_tick() {
        // RTT 80 ms with +/-10 ms jitter, one snapshot per server tick for two seconds.
        let mut estimator = TickEstimator::default();
        let mut rng = SplitMix64::new(42);
        for tick in 0..60u64 {
            estimator.advance(SERVER_TICK_DURATION);
            let jitter_ms = rng.next_unit() * 20.0 - 10.0;
            let rtt = Duration::from_secs_f64((80.0 + jitter_ms) / 1000.0);
            estimator.observe(tick, rtt);
        }
        // The last snapshot was taken at tick 59 and arrived 40 ms later, which is tick 60.2.
        let estimate = estimator.estimate().expect("estimate exists after snapshots");
        assert!(estimate.abs_diff(60) <= 1, "estimate was {estimate}");
    }
}
