//! Material description for surface appearance (including Substrate lobes).

use redox_math::Vec3;

use crate::asset_types::MaterialLayer;

/// Describes the visual properties of a surface.
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
    /// Optional Substrate-style layers (up to two). `None` = single-lobe.
    pub layers: Option<[MaterialLayer; 2]>,
    /// Sheen lobe weight for fabric-like surfaces (0 = disabled).
    pub sheen: f32,
}

/// GPU material uniform — must stay 16-byte aligned (wgpu validation is fatal).
///
/// Layout (112 bytes):
/// * base_color / emissive (2× vec4)
/// * metallic, roughness, flags, sheen
/// * layer0/1 color (rgb + metallic) + params (roughness, weight, pad, pad)
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialUniform {
    pub base_color: [f32; 4],
    pub emissive: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub flags: u32,
    pub sheen: f32,
    pub layer0_color: [f32; 4],
    pub layer0_params: [f32; 4],
    pub layer1_color: [f32; 4],
    pub layer1_params: [f32; 4],
}

pub mod material_flags {
    pub const HAS_ALBEDO_TEX: u32 = 1 << 0;
    pub const HAS_NORMAL_TEX: u32 = 1 << 1;
    pub const HAS_MR_TEX: u32 = 1 << 2;
    /// Substrate secondary lobes are active (any layer weight > 0 or sheen > 0).
    pub const HAS_SUBSTRATE: u32 = 1 << 3;
    /// Enable stochastic alpha-to-coverage discard path.
    pub const ALPHA_TEST: u32 = 1 << 4;
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
            layers: None,
            sheen: 0.0,
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
            layers: None,
            sheen: 0.0,
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

    pub fn with_layers(mut self, layers: [MaterialLayer; 2]) -> Self {
        self.layers = Some(layers);
        self
    }

    pub fn sheen(mut self, sheen: f32) -> Self {
        self.sheen = sheen;
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

        let (l0, l1) = match &self.layers {
            Some([a, b]) => (*a, *b),
            None => (MaterialLayer::default(), MaterialLayer::default()),
        };
        if self.sheen > 1e-4 || l0.weight > 1e-4 || l1.weight > 1e-4 {
            flags |= material_flags::HAS_SUBSTRATE;
        }

        MaterialUniform {
            base_color: [self.base_color.x, self.base_color.y, self.base_color.z, 1.0],
            emissive: [self.emissive.x, self.emissive.y, self.emissive.z, 0.0],
            metallic: self.metallic,
            roughness: self.roughness,
            flags,
            sheen: self.sheen,
            layer0_color: [l0.base_color.x, l0.base_color.y, l0.base_color.z, l0.metallic],
            layer0_params: [l0.roughness, l0.weight, 0.0, 0.0],
            layer1_color: [l1.base_color.x, l1.base_color.y, l1.base_color.z, l1.metallic],
            layer1_params: [l1.roughness, l1.weight, 0.0, 0.0],
        }
    }
}

impl Default for Material {
    /// Default: opaque white, no texture.
    fn default() -> Self {
        Self::solid(Vec3::ONE)
    }
}

const _: () = {
    assert!(std::mem::size_of::<MaterialUniform>() == 112);
};
