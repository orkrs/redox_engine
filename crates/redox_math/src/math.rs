//! Mathematical foundation for the RedOx Engine.
//!
//! This crate provides linear algebra types (vectors, matrices, quaternions)
//! re-exported from [`glam`], along with geometric primitives (`Aabb`, `Sphere`)
//! and frustum culling utilities. All types are designed for high-performance
//! 3D graphics and physics simulations.

pub mod vector;
pub mod matrix;
pub mod quat;
pub mod bounds;
pub mod frustum;
pub mod conversions;
pub mod transform;

// Re-export all public items from submodules for convenient glob imports.
pub use vector::*;
pub use matrix::*;
pub use quat::*;
pub use bounds::*;
pub use frustum::*;
pub use conversions::*;
pub use transform::*;