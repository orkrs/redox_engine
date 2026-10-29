//! Local reflection probes: box-shaped influence volumes with parallax-corrected cubemap lookups.

mod capture;
mod component;
mod gpu;
mod probe;
mod set;

pub use component::{DEFAULT_PROBE_RESOLUTION, ReflectionProbeComponent, is_valid_resolution};
pub use gpu::{ProbeGpu, ProbesGpu};
pub use probe::{ReflectionProbe, parallax_direction, probe_weight, select_probes};
pub use set::{MAX_PROBES, PROBE_FACE_SIZE, PROBE_FORMAT, PROBE_MIP_LEVELS, ReflectionProbeSet};
