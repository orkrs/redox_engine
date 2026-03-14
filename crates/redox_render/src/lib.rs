//! Rendering subsystem for the RedOx Engine.
//!
//! This crate provides a `wgpu`-based renderer with forward shading,
//! procedural mesh generation, texture loading, and ECS integration
//! through the `redox_ecs` crate.

pub mod asset_types;
pub mod camera;
pub mod clustering;
pub mod cluster_manager;
pub mod context;
pub mod light;
pub mod material;
pub mod mesh;
pub mod pass;
pub mod post;
pub mod resource;
pub mod shader;
pub mod systems;

pub use camera::{ActiveCamera, Camera, CameraUniform};
pub use context::RenderContext;
pub use light::{DirectionalLight, LightUniform, PointLight, PointLightGpu};
pub use material::Material;
pub use asset_types::{MaterialData, MeshData, TextureData};
pub use mesh::{Mesh, Vertex};
pub use systems::{MaterialHandle, MeshHandle, RenderObject, Transform};
pub use clustering::ClusterInfo;
pub use cluster_manager::{ClusterManager, ClusterAssignmentStats};
