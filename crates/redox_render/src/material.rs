//! Material description for surface appearance (including Substrate lobes).
//!
//! Compatibility (MAT-04/MAT-05): a `Material` built with the existing
//! constructors has no coat, no anisotropy and no subsurface profile. Its GPU
//! flags are then identical to before, and the shader takes the legacy path,
//! so the frame stays bit-identical. The new fields are appended to the GPU
//! uniform; the legacy fields keep their offsets.

use redox_math::Vec3;

use crate::asset_types::MaterialLayer;

/// Subsurface-scattering profile for wrap lighting (MAT-05).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubsurfaceProfile {
    /// Scattering colour (linear RGB), tints the shadow terminator.
    pub color: [f32; 3],
    /// Scattering radius in millimetres (0..20).
    pub radius_mm: f32,
    /// Wrap factor (0..1): 0 = plain Lambert, 1 = fully wrapped lighting.
    pub wrap: f32,
}

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
    /// Clearcoat weight (0 = no coat).
    pub coat_weight: f32,
    /// Clearcoat perceptual roughness.
    pub coat_roughness: f32,
    /// Clearcoat index of refraction (1.5 by default).
    pub coat_ior: f32,
    /// Clearcoat transmittance tint (linear RGB).
    pub coat_tint: Vec3,
    /// Anisotropy of the base specular lobe, -1..1 (0 = isotropic).
    pub anisotropy: f32,
    /// Subsurface wrap profile (`None` = no subsurface).
    pub subsurface: Option<SubsurfaceProfile>,
}

/// GPU material uniform — must stay 16-byte aligned (wgpu validation is fatal).
///
/// Layout (176 bytes):
/// * base_color / emissive (2× vec4)
/// * metallic, roughness, flags, sheen
/// * layer0/1 color (rgb + metallic) + params (roughness, weight, pad, pad)
/// * coat params (weight, roughness, ior, anisotropy), coat tint (rgb, pad)
/// * subsurface colour (rgb, radius mm), subsurface params (wrap, pad, pad, pad)
///
/// The coat/anisotropy/subsurface rows are appended; `pbr.wgsl` and
/// `vg_resolve.wgsl` mirror this layout.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialUniform {
    /// Base color (rgba).
    pub base_color: [f32; 4],
    /// Emissive color (rgb) and strength.
    pub emissive: [f32; 4],
    /// Metallic factor.
    pub metallic: f32,
    /// Roughness.
    pub roughness: f32,
    /// Bit set of [`material_flags`].
    pub flags: u32,
    /// Sheen strength.
    pub sheen: f32,
    /// Color (rgb) and metallic factor (w) of the first layer.
    pub layer0_color: [f32; 4],
    /// Roughness (x) and blend weight (y) of the first layer.
    pub layer0_params: [f32; 4],
    /// Color (rgb) and metallic factor (w) of the second layer.
    pub layer1_color: [f32; 4],
    /// Roughness (x) and blend weight (y) of the second layer.
    pub layer1_params: [f32; 4],
    /// Coat weight (x), coat roughness (y), coat IOR (z), anisotropy (w).
    pub coat_params: [f32; 4],
    /// Coat transmittance tint (rgb), pad (w).
    pub coat_tint: [f32; 4],
    /// Subsurface scatter colour (rgb), scatter radius in mm (w).
    pub sss_color: [f32; 4],
    /// Subsurface wrap factor (x), pad (yzw).
    pub sss_params: [f32; 4],
}

/// Bit flags for [`MaterialUniform::flags`].
pub mod material_flags {
    /// The material has an albedo texture.
    pub const HAS_ALBEDO_TEX: u32 = 1 << 0;
    /// The material has a normal map.
    pub const HAS_NORMAL_TEX: u32 = 1 << 1;
    /// The material has a metallic-roughness texture.
    pub const HAS_MR_TEX: u32 = 1 << 2;
    /// Substrate secondary lobes are active (any layer weight > 0 or sheen > 0).
    pub const HAS_SUBSTRATE: u32 = 1 << 3;
    /// Enable stochastic alpha-to-coverage discard path.
    pub const ALPHA_TEST: u32 = 1 << 4;
    /// Clearcoat is active (`coat_params.x > 0`).
    pub const HAS_COAT: u32 = 1 << 5;
    /// Anisotropic base specular is active (`coat_params.w != 0`).
    pub const HAS_ANISOTROPY: u32 = 1 << 6;
    /// Subsurface wrap lighting is active (`sss_params.x > 0`).
    pub const HAS_SUBSURFACE: u32 = 1 << 7;
}

/// Coat IOR used when a coat is enabled without an explicit IOR.
const DEFAULT_COAT_IOR: f32 = 1.5;

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
            coat_weight: 0.0,
            coat_roughness: 0.05,
            coat_ior: DEFAULT_COAT_IOR,
            coat_tint: Vec3::ONE,
            anisotropy: 0.0,
            subsurface: None,
        }
    }

    /// Creates a textured material.
    pub fn textured(color: Vec3, texture_index: usize) -> Self {
        Self {
            texture_index: Some(texture_index),
            ..Self::solid(color)
        }
    }

    /// Sets the metallic factor.
    pub fn metallic(mut self, metallic: f32) -> Self {
        self.metallic = metallic;
        self
    }

    /// Sets the roughness.
    pub fn roughness(mut self, roughness: f32) -> Self {
        self.roughness = roughness;
        self
    }

    /// Sets the emissive color.
    pub fn emissive(mut self, color: Vec3) -> Self {
        self.emissive = color;
        self
    }

    /// Uses the texture at `texture_index` as normal map.
    pub fn with_normal(mut self, texture_index: usize) -> Self {
        self.normal_texture_index = Some(texture_index);
        self
    }

    /// Uses the texture at `texture_index` as metallic-roughness map.
    pub fn with_mr(mut self, texture_index: usize) -> Self {
        self.mr_texture_index = Some(texture_index);
        self
    }

    /// Sets the two material layers.
    pub fn with_layers(mut self, layers: [MaterialLayer; 2]) -> Self {
        self.layers = Some(layers);
        self
    }

    /// Sets the sheen strength.
    pub fn sheen(mut self, sheen: f32) -> Self {
        self.sheen = sheen;
        self
    }

    /// Adds a clearcoat with the given weight and roughness (IOR 1.5, no tint).
    pub fn coat(mut self, weight: f32, roughness: f32) -> Self {
        self.coat_weight = weight;
        self.coat_roughness = roughness;
        self
    }

    /// Sets the anisotropy of the base specular lobe (-1..1).
    pub fn anisotropy(mut self, anisotropy: f32) -> Self {
        self.anisotropy = anisotropy;
        self
    }

    /// Enables subsurface wrap lighting with `profile`.
    pub fn subsurface(mut self, profile: SubsurfaceProfile) -> Self {
        self.subsurface = Some(profile);
        self
    }

    /// Converts to the GPU uniform layout.
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

        let has_coat = self.coat_weight > 0.0;
        if has_coat {
            flags |= material_flags::HAS_COAT;
        }
        if self.anisotropy.abs() > 1e-6 {
            flags |= material_flags::HAS_ANISOTROPY;
        }
        let (sss_color, sss_params) = match &self.subsurface {
            Some(p) if p.wrap > 0.0 => {
                flags |= material_flags::HAS_SUBSURFACE;
                (
                    [p.color[0], p.color[1], p.color[2], p.radius_mm],
                    [p.wrap, 0.0, 0.0, 0.0],
                )
            }
            _ => ([0.0; 4], [0.0; 4]),
        };

        MaterialUniform {
            base_color: [self.base_color.x, self.base_color.y, self.base_color.z, 1.0],
            emissive: [self.emissive.x, self.emissive.y, self.emissive.z, 0.0],
            metallic: self.metallic,
            roughness: self.roughness,
            flags,
            sheen: self.sheen,
            layer0_color: [
                l0.base_color.x,
                l0.base_color.y,
                l0.base_color.z,
                l0.metallic,
            ],
            layer0_params: [l0.roughness, l0.weight, 0.0, 0.0],
            layer1_color: [
                l1.base_color.x,
                l1.base_color.y,
                l1.base_color.z,
                l1.metallic,
            ],
            layer1_params: [l1.roughness, l1.weight, 0.0, 0.0],
            coat_params: [
                self.coat_weight,
                self.coat_roughness,
                self.coat_ior,
                self.anisotropy,
            ],
            coat_tint: [self.coat_tint.x, self.coat_tint.y, self.coat_tint.z, 0.0],
            sss_color,
            sss_params,
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
    assert!(std::mem::size_of::<MaterialUniform>() == 176);
};
