//! Material description for surface appearance.

use redox_math::Vec3;

/// Describes the visual properties of a surface.
///
/// In the MVP only `base_color` and an optional texture are used.
/// `metallic` and `roughness` are stored for future PBR expansion.
#[derive(Clone, Debug)]
pub struct Material {
    /// Base albedo colour (linear RGB).
    pub base_color: Vec3,
    /// Optional index into the texture storage for albedo.
    pub texture_index: Option<usize>,
    /// Optional index into the texture storage for normal map.
    pub normal_texture_index: Option<usize>,
    /// Optional index into the texture storage for metallic-roughness map.
    pub mr_texture_index: Option<usize>,
    /// Metallic factor (0.0 = dielectric, 1.0 = metal).
    pub metallic: f32,
    /// Roughness factor (0.0 = mirror, 1.0 = rough).
    pub roughness: f32,
    /// Emissive colour.
    pub emissive: Vec3,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialUniform {
    pub base_color: [f32; 4],
    pub emissive: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub flags: u32,
    pub _padding: u32,
}

pub mod material_flags {
    pub const HAS_ALBEDO_TEX: u32 = 1 << 0;
    pub const HAS_NORMAL_TEX: u32 = 1 << 1;
    pub const HAS_MR_TEX: u32 = 1 << 2;
}

impl Material {
    /// Creates a simple solid-colour material.
    pub fn solid(color: Vec3) -> Self {
        Self {
            base_color: color,
            texture_index: None,
            normal_texture_index: None,
            mr_texture_index: None,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Vec3::ZERO,
        }
    }

    /// Creates a textured material.
    pub fn textured(color: Vec3, texture_index: usize) -> Self {
        Self {
            base_color: color,
            texture_index: Some(texture_index),
            normal_texture_index: None,
            mr_texture_index: None,
            metallic: 0.0,
            roughness: 0.5,
            emissive: Vec3::ZERO,
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

    pub fn with_normal(mut self, texture_index: usize) -> Self {
        self.normal_texture_index = Some(texture_index);
        self
    }

    pub fn with_mr(mut self, texture_index: usize) -> Self {
        self.mr_texture_index = Some(texture_index);
        self
    }

    pub fn to_uniform(&self, _device: &wgpu::Device) -> MaterialUniform {
        let mut flags = 0;
        if self.texture_index.is_some() {
            flags |= material_flags::HAS_ALBEDO_TEX;
        }
        if self.normal_texture_index.is_some() {
            flags |= material_flags::HAS_NORMAL_TEX;
        }
        if self.mr_texture_index.is_some() {
            flags |= material_flags::HAS_MR_TEX;
        }

        MaterialUniform {
            base_color: [self.base_color.x, self.base_color.y, self.base_color.z, 1.0],
            emissive: [self.emissive.x, self.emissive.y, self.emissive.z, 0.0],
            metallic: self.metallic,
            roughness: self.roughness,
            flags,
            _padding: 0,
        }
    }
}

impl Default for Material {
    /// Default: opaque white, no texture.
    fn default() -> Self {
        Self::solid(Vec3::ONE)
    }
}
