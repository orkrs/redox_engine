//! Global illumination: DDGI scaffold (to be replaced), octahedral and cascade helpers,
//! the analytic CPU reference, the renderer-side SDF state, and the GPU probe ray trace.

pub mod atlas;
pub mod cascade;
pub mod ddgi;
pub mod ddgi_gpu;
pub mod grid_gpu;
pub mod oct;
pub mod probes;
pub mod reference;
pub mod sdf_ref;
pub mod state;
pub mod trace;

pub use ddgi::{DdgiProbe, DdgiProbeGrid, GridDimensions};
pub use ddgi_gpu::{DdgiGpu, DdgiGridGpu, DdgiProbeGpu};
