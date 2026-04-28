//! Shadow subsystem.
//!
//! Historically this module contained the Virtual Shadow Maps (VSM)
//! implementation. The engine has since pivoted to a simpler and more
//! robust cascaded-shadow setup. The remaining modules are classic
//! depth-map utilities that will be reused by the new CSM path.

pub mod csm;
