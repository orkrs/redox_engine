//! Directional light component and GPU uniform.
//!
//! Supports both legacy per-vertex light arrays and modern clustered forward rendering.

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

/// ECS component for a directional (sun-like) light source.
///
/// In the MVP only one directional light is supported. The `direction` vector
/// points *towards* the light — i.e., opposite to the direction the light
/// rays travel.
#[derive(Clone, Debug)]
pub struct DirectionalLight {
    /// Normalised direction **towards** the light source.
    pub direction: Vec3,
    /// Light colour (linear RGB, not sRGB).
    pub color: Vec3,
    /// Intensity multiplier.
    pub intensity: f32,
}

impl DirectionalLight {
    /// Creates a new directional light.
    pub fn new(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            direction: direction.normalize(),
            color,
            intensity,
        }
    }
}

impl Default for DirectionalLight {
    /// Default: white light coming from above and slightly to the side.
    fn default() -> Self {
        Self::new(Vec3::new(0.3, 1.0, 0.5), Vec3::ONE, 1.0)
    }
}

/// ECS component for a point light source.
#[derive(Clone, Debug)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
}

impl PointLight {
    pub fn new(position: Vec3, color: Vec3, intensity: f32, radius: f32) -> Self {
        Self {
            position,
            color,
            intensity,
            radius,
        }
    }
}

/// GPU-friendly light parameters for multiple lights.
///
/// Supports 1 directional light and up to 128 point lights.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LightUniform {
    // Directional light: xyz = direction, w = intensity
    pub dir_color: [f32; 4],
    pub dir_direction: [f32; 4],

    // Ambient color: xyz = color, w = ambient intensity (1.0 = full)
    pub ambient: [f32; 4],

    // Shadow matrix
    pub shadow_view_proj: [[f32; 4]; 4],

    // Point lights (array of 128 for a more atmospheric forest)
    // pos.xyz, pos.w = intensity; color.xyz, color.w = radius
    pub point_lights_pos: [[f32; 4]; 128],
    pub point_lights_color: [[f32; 4]; 128],

    pub num_point_lights: u32,
    pub _padding: [u32; 3], // 16-byte alignment
}

impl LightUniform {
    pub fn new(dir_light: &DirectionalLight, ambient: Vec3) -> Self {
        let dc = dir_light.color * dir_light.intensity;
        Self {
            dir_color: [dc.x, dc.y, dc.z, dir_light.intensity],
            dir_direction: [
                dir_light.direction.x,
                dir_light.direction.y,
                dir_light.direction.z,
                0.0,
            ],
            ambient: [ambient.x, ambient.y, ambient.z, 1.0], // w = 1.0 — ambient multiplier
            shadow_view_proj: [[0.0; 4]; 4],
            point_lights_pos: [[0.0; 4]; 128],
            point_lights_color: [[0.0; 4]; 128],
            num_point_lights: 0,
            _padding: [0; 3],
        }
    }

    pub fn add_point_light(&mut self, light: &PointLight) {
        if self.num_point_lights < 128 {
            let i = self.num_point_lights as usize;
            self.point_lights_pos[i] = [
                light.position.x,
                light.position.y,
                light.position.z,
                light.intensity,
            ];
            self.point_lights_color[i] =
                [light.color.x, light.color.y, light.color.z, light.radius];
            self.num_point_lights += 1;
        }
    }
}

impl Default for LightUniform {
    fn default() -> Self {
        Self::new(&DirectionalLight::default(), Vec3::splat(0.15))
    }
}

/// GPU-friendly point light structure for storage buffers (clustered rendering).
///
/// This is used instead of the fixed arrays in LightUniform for better scaling.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PointLightGpu {
    /// Position in world space (padded to vec4).
    pub position: [f32; 4],
    /// Color in linear RGB (padded to vec4).
    pub color: [f32; 4],
    /// Intensity multiplier.
    pub intensity: f32,
    /// Attenuation radius.
    pub radius: f32,
    /// Unused padding.
    pub _padding: [f32; 2],
}

impl PointLightGpu {
    /// Creates a GPU light from a CPU light component.
    pub fn from_point_light(light: &PointLight) -> Self {
        Self {
            position: [light.position.x, light.position.y, light.position.z, 1.0],
            color: [light.color.x, light.color.y, light.color.z, 1.0],
            intensity: light.intensity,
            radius: light.radius,
            _padding: [0.0; 2],
        }
    }
}