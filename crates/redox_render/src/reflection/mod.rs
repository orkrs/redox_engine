//! Local reflection probes: box-shaped influence volumes with parallax-corrected cubemap lookups.

mod probe;

pub use probe::{ReflectionProbe, parallax_direction, probe_weight, select_probes};
