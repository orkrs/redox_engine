//! Asset data types used with [`redox_asset`] for render resources.
//!
//! These types are stored in the asset manager; when loaded (or inserted),
//! the render context creates GPU resources and maps handles to indices.

use redox_asset::Handle;
use redox_math::Vec3;

use crate::mesh::Mesh;

/// CPU-side mesh data. Stored in the asset manager; uploaded to GPU when synced.
pub type MeshData = Mesh;

/// Image data for textures. Produced by [`redox_asset::ImageLoader`].
pub type TextureData = image::DynamicImage;

/// Material descriptor (no GPU indices). Stored in the asset manager.
///
/// Texture slots reference other assets by handle; when creating the GPU material,
/// the render context resolves these to texture indices.
#[derive(Clone, Debug)]
pub struct MaterialData {
    pub base_color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    pub albedo_handle: Option<Handle<TextureData>>,
    pub normal_handle: Option<Handle<TextureData>>,
    pub mr_handle: Option<Handle<TextureData>>,
}

impl MaterialData {
    pub fn solid(color: Vec3) -> Self {
        Self {
            base_color: color,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Vec3::ZERO,
            albedo_handle: None,
            normal_handle: None,
            mr_handle: None,
        }
    }

    pub fn metallic(mut self, metallic: f32) -> Self {
        self.metallic = metallic;
        self
    }

    pub fn roughness(mut self, roughness: f32) -> Self {
        self.roughness = roughness;
        self
    }

    pub fn emissive(mut self, color: Vec3) -> Self {
        self.emissive = color;
        self
    }

    pub fn with_albedo(mut self, handle: Handle<TextureData>) -> Self {
        self.albedo_handle = Some(handle);
        self
    }

    pub fn with_normal(mut self, handle: Handle<TextureData>) -> Self {
        self.normal_handle = Some(handle);
        self
    }

    pub fn with_mr(mut self, handle: Handle<TextureData>) -> Self {
        self.mr_handle = Some(handle);
        self
    }
}
