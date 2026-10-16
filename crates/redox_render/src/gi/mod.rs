//! Global illumination modules (DDGI scaffold + GPU upload).

pub mod ddgi;
pub mod ddgi_gpu;

pub use ddgi::{DdgiProbe, DdgiProbeGrid, GridDimensions};
pub use ddgi_gpu::{DdgiGpu, DdgiGridGpu, DdgiProbeGpu};
