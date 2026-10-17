//! Mathematical foundation for the RedOx Engine.
//!
//! This crate provides linear algebra types (vectors, matrices, quaternions)
//! re-exported from [`glam`], along with geometric primitives (`Aabb`, `Sphere`)
//! and frustum culling utilities. All types are designed for high-performance
//! 3D graphics and physics simulations.

#![warn(missing_docs)]

/// Axis-aligned bounding boxes.
pub mod bounds;
pub mod frustum;
/// Matrix helpers: view and projection constructors.
pub mod matrix;
/// Quaternion helpers.
pub mod quat;
/// `Transform` and `GlobalTransform` components.
pub mod transform;
pub mod vector;

// Re-export all public items from submodules for convenient glob imports.
pub use bounds::*;
pub use frustum::*;
pub use matrix::*;
pub use quat::*;
pub use transform::*;
pub use vector::*;
