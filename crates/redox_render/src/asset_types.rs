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

/// Substrate-style material layer (clear coat, secondary BRDF lobe, etc.).
#[derive(Clone, Copy, Debug)]
pub struct MaterialLayer {
    pub base_color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub weight: f32,
}

impl Default for MaterialLayer {
    fn default() -> Self {
        Self {
            base_color: Vec3::ONE,
            metallic: 0.0,
            roughness: 0.5,
            weight: 0.0,
        }
    }
}

/// How a material's surface is composited into the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    /// Fully opaque surface (default): rendered in the main forward pass.
    #[default]
    Opaque,
    /// Alpha-blended surface: rendered after all opaque geometry, sorted
    /// back-to-front, without writing depth.
    Alpha,
}

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
    /// How the surface is blended into the frame (default [`BlendMode::Opaque`]).
    pub blend: BlendMode,
    /// Optional Substrate-style layers (up to two). `None` = single-lobe material.
    pub layers: Option<[MaterialLayer; 2]>,
    /// Sheen lobe weight for fabric-like surfaces (0 = disabled).
    pub sheen: f32,
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
            blend: BlendMode::Opaque,
            layers: None,
            sheen: 0.0,
        }
    }

    /// Sets the blend mode (builder style).
    pub fn blend(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
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

    /// Enables Substrate-style dual layers (builder style).
    pub fn with_layers(mut self, layers: [MaterialLayer; 2]) -> Self {
        self.layers = Some(layers);
        self
    }

    /// Sets sheen weight (builder style).
    pub fn sheen(mut self, sheen: f32) -> Self {
        self.sheen = sheen;
        self
    }
}
