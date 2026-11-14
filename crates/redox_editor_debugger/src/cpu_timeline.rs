//! CPU timeline model (reads dispatcher timings without egui).

use redox_core::dispatcher::SystemTiming;

/// One bar in the profiler CPU timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuSpan {
    /// System name.
    pub name: String,
    /// Duration in milliseconds.
    pub ms: f32,
}

/// Timeline for one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuTimeline {
    /// Spans in execution order.
    pub spans: Vec<CpuSpan>,
}

impl CpuTimeline {
    /// Builds a timeline from dispatcher timings (empty when flag was off).
    pub fn from_system_timings(timings: &[SystemTiming]) -> Self {
        Self {
            spans: timings
                .iter()
                .map(|t| CpuSpan {
                    name: t.name.clone(),
                    ms: t.duration_ns as f32 / 1.0e6,
                })
                .collect(),
        }
    }

    /// Total CPU time in milliseconds.
    pub fn total_ms(&self) -> f32 {
        self.spans.iter().map(|s| s.ms).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_core::dispatcher::Stage;

    #[test]
    fn from_timings_preserves_order_and_ms() {
        let timings = vec![
            SystemTiming {
                stage: Stage::Update,
                name: "a".into(),
                duration_ns: 2_000_000,
            },
            SystemTiming {
                stage: Stage::Update,
                name: "b".into(),
                duration_ns: 500_000,
            },
        ];
        let tl = CpuTimeline::from_system_timings(&timings);
        assert_eq!(tl.spans.len(), 2);
        assert!((tl.spans[0].ms - 2.0).abs() < 0.01);
    }
}