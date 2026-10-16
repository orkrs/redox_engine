//! Directional light component and GPU uniform.
//!
//! Supports both legacy per-vertex light arrays and modern clustered forward rendering.

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

/// Forward lighting path selection (mirrors `redox_core::LightingMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightingMode {
    /// Clustered forward shading (default).
    #[default]
    Clustered,
    /// ScatterLight — RedOx stochastic per-pixel point-light path (K=8).
    ScatterLight,
}

/// ECS component for a directional (sun-like) light source.
///
/// Photometric units: `intensity` in **lux** (illuminance at surface),
/// `color` is linear RGB chromaticity. Combined with `TonemapUniform.exposure`
/// (EV100) this yields a unified physical scale with point lights (lumens/candela).
/// `source_angle` drives SMRT penumbra in VSM path.
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
    /// Angular radius of the source disc (radians).  Used by SMRT for
    /// soft shadow penumbra width.  Default ~0.53° for a sun-like source.
    pub source_angle: f32,
    /// Enables the PCSS soft-shadow path in the PBR shader.
    pub soft_shadows: bool,
    /// Routes directional shadows through the VSM clipmap (soft_shadows=2) instead of PCSS (1).
    pub vsm: bool,
}

impl DirectionalLight {
    /// Creates a new directional light.
    pub fn new(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            direction: direction.normalize(),
            color,
            intensity,
            source_angle: 0.0087,
            soft_shadows: false,
            vsm: true,
        }
    }
}

impl Default for DirectionalLight {
    /// Default: white light coming from above and slightly to the side.
    fn default() -> Self {
        Self::new(Vec3::new(0.3, 1.0, 0.5), Vec3::ONE, 1.0)
    }
}

impl redox_reflect::Reflect for DirectionalLight {
    fn type_name() -> &'static str {
        "DirectionalLight"
    }
    fn to_value(&self) -> redox_reflect::ReflectValue {
        use redox_reflect::ReflectValue;
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![
                (
                    "direction".into(),
                    ReflectValue::vec3(self.direction.x, self.direction.y, self.direction.z),
                ),
                (
                    "color".into(),
                    ReflectValue::vec3(self.color.x, self.color.y, self.color.z),
                ),
                ("intensity".into(), ReflectValue::f32(self.intensity)),
                ("source_angle".into(), ReflectValue::f32(self.source_angle)),
                ("soft_shadows".into(), ReflectValue::Bool(self.soft_shadows)),
                ("vsm".into(), ReflectValue::Bool(self.vsm)),
            ],
        }
    }
    fn apply(
        &mut self,
        value: &redox_reflect::ReflectValue,
    ) -> Result<(), redox_reflect::ReflectError> {
        if let Some(v) = value
            .field("direction")
            .and_then(redox_reflect::ReflectValue::as_vec3)
        {
            self.direction = Vec3::from_array(v).normalize_or_zero();
        }
        if let Some(v) = value
            .field("color")
            .and_then(redox_reflect::ReflectValue::as_vec3)
        {
            self.color = Vec3::from_array(v);
        }
        if let Some(v) = value
            .field("intensity")
            .and_then(redox_reflect::ReflectValue::as_f32)
        {
            self.intensity = v;
        }
        if let Some(v) = value
            .field("source_angle")
            .and_then(redox_reflect::ReflectValue::as_f32)
        {
            self.source_angle = v;
        }
        if let Some(redox_reflect::ReflectValue::Bool(b)) = value.field("soft_shadows") {
            self.soft_shadows = *b;
        }
        if let Some(redox_reflect::ReflectValue::Bool(b)) = value.field("vsm") {
            self.vsm = *b;
        }
        Ok(())
    }
    fn reflect_schema() -> redox_reflect::ReflectSchema {
        redox_reflect::ReflectSchema {
            type_name: Self::type_name(),
            fields: vec![
                redox_reflect::ReflectField {
                    name: "direction",
                    type_name: "Vec3",
                },
                redox_reflect::ReflectField {
                    name: "color",
                    type_name: "Vec3",
                },
                redox_reflect::ReflectField {
                    name: "intensity",
                    type_name: "f32",
                },
            ],
        }
    }
}

/// ECS component for a point light source.
///
/// Photometric units: `intensity` in **lumens** (or candela when divided by 4π),
/// `color` linear RGB, `radius` defines Frostbite smooth window falloff
/// (inverse-square + `clip(1-(d/r)^4)^2`). Keeps unified scale with sun lux + EV100 exposure.
#[derive(Clone, Debug)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Vec3,
    /// Intensity in lumens (point) — scaled by windowed inverse-square in shader.
    pub intensity: f32,
    /// Radius where smooth window reaches zero (no hard step).
    pub radius: f32,
    /// Whether this light casts cube-map shadows (atlas tiles).
    pub cast_shadows: bool,
    /// Physical radius of the light emitter for SMRT soft shadows.
    pub source_radius: f32,
}

impl PointLight {
    pub fn new(position: Vec3, color: Vec3, intensity: f32, radius: f32) -> Self {
        Self {
            position,
            color,
            intensity,
            radius,
            cast_shadows: false,
            source_radius: 0.05,
        }
    }
}

impl Default for PointLight {
    fn default() -> Self {
        Self::new(Vec3::ZERO, Vec3::ONE, 1.0, 8.0)
    }
}

impl redox_reflect::Reflect for PointLight {
    fn type_name() -> &'static str {
        "PointLight"
    }

    fn to_value(&self) -> redox_reflect::ReflectValue {
        use redox_reflect::ReflectValue;
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![
                (
                    "position".into(),
                    ReflectValue::vec3(self.position.x, self.position.y, self.position.z),
                ),
                (
                    "color".into(),
                    ReflectValue::vec3(self.color.x, self.color.y, self.color.z),
                ),
                ("intensity".into(), ReflectValue::f32(self.intensity)),
                ("radius".into(), ReflectValue::f32(self.radius)),
                ("cast_shadows".into(), ReflectValue::Bool(self.cast_shadows)),
                (
                    "source_radius".into(),
                    ReflectValue::f32(self.source_radius),
                ),
            ],
        }
    }

    fn apply(
        &mut self,
        value: &redox_reflect::ReflectValue,
    ) -> Result<(), redox_reflect::ReflectError> {
        if let Some(v) = value
            .field("position")
            .and_then(redox_reflect::ReflectValue::as_vec3)
        {
            self.position = Vec3::from_array(v);
        }
        if let Some(v) = value
            .field("color")
            .and_then(redox_reflect::ReflectValue::as_vec3)
        {
            self.color = Vec3::from_array(v);
        }
        if let Some(v) = value
            .field("intensity")
            .and_then(redox_reflect::ReflectValue::as_f32)
        {
            self.intensity = v;
        }
        if let Some(v) = value
            .field("radius")
            .and_then(redox_reflect::ReflectValue::as_f32)
        {
            self.radius = v;
        }
        if let Some(redox_reflect::ReflectValue::Bool(b)) = value.field("cast_shadows") {
            self.cast_shadows = *b;
        }
        if let Some(v) = value
            .field("source_radius")
            .and_then(redox_reflect::ReflectValue::as_f32)
        {
            self.source_radius = v;
        }
        Ok(())
    }
}

/// ECS component for a spot light source.
#[derive(Clone, Debug)]
pub struct SpotLight {
    pub position: Vec3,
    pub direction: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
    /// Outer cone half-angle in radians.
    pub outer_cone_angle: f32,
    /// Inner cone half-angle in radians (full brightness).
    pub inner_cone_angle: f32,
    pub cast_shadows: bool,
    pub source_radius: f32,
}

impl SpotLight {
    pub fn new(
        position: Vec3,
        direction: Vec3,
        color: Vec3,
        intensity: f32,
        range: f32,
        outer_cone_angle: f32,
    ) -> Self {
        Self {
            position,
            direction: direction.normalize(),
            color,
            intensity,
            range,
            outer_cone_angle,
            inner_cone_angle: outer_cone_angle * 0.8,
            cast_shadows: false,
            source_radius: 0.05,
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
    /// 1 = PCSS/VSM for directional; 2 = VSM clipmap path.
    pub soft_shadows: u32,
    /// 0 = clustered point lights in PBR; 1 = ScatterLight pass owns points.
    pub point_light_path: u32,
    /// 0 = inline VSM multi-ray in PBR; 1 = screen-space 1-ray + temporal (filtered texture).
    pub vsm_screen_shadows: u32,
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
            soft_shadows: if dir_light.vsm {
                2u32
            } else {
                u32::from(dir_light.soft_shadows)
            },
            point_light_path: 0,
            vsm_screen_shadows: 0,
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
        Self::new(&DirectionalLight::default(), Vec3::splat(0.02))
    }
}

/// GPU-friendly point light structure for storage buffers (clustered rendering).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PointLightGpu {
    /// Position in world space (padded to vec4).
    pub position: [f32; 4],
    /// Color in linear RGB (padded to vec4).
    pub color: [f32; 4],
    /// Shadow view-projection matrices for point-light cube faces.
    /// Face order matches the WGSL `cube_face_index` mapping.
    pub shadow_matrices: [[[f32; 4]; 4]; 6],
    /// Intensity multiplier.
    pub intensity: f32,
    /// Attenuation radius.
    pub radius: f32,
    /// First atlas tile of this light's 6-tile run, or `shadow::NO_TILE`.
    pub shadow_tile_base: u32,
    /// Unused padding.
    pub _padding: [f32; 1],
}

/// Shader debug uniform (binding 21): debug_viz_mode and padding for WGSL layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct ShaderDebugUniform {
    pub debug_viz_mode: u32,
    pub frame_index: u32,
    /// 0 while dummy IBL is bound — PBR must not sample uninitialized cubemaps.
    pub ibl_enabled: u32,
    pub gtao_enabled: u32,
}

impl PointLightGpu {
    /// Creates a GPU light from a CPU light component.
    pub fn from_point_light(light: &PointLight) -> Self {
        Self {
            position: [light.position.x, light.position.y, light.position.z, 1.0],
            color: [light.color.x, light.color.y, light.color.z, 1.0],
            shadow_matrices: [redox_math::Mat4::IDENTITY.to_cols_array_2d(); 6],
            intensity: light.intensity,
            radius: light.radius,
            // Overwritten by the renderer once atlas tiles are allocated.
            shadow_tile_base: crate::shadow::NO_TILE,
            _padding: [0.0],
        }
    }
}

// ── Scene-level lighting ─────────────────────────────────────────────────────

/// CPU-side scene lighting collected each frame.
///
/// This is the single input to the renderer's lighting path; the legacy
/// [`LightUniform`] / [`PointLightGpu`] conversions are driven from here.
#[derive(Clone, Debug)]
pub struct LightScene {
    /// The sun. Only one directional light is supported.
    pub directional: DirectionalLight,
    /// Ambient light color added to every surface.
    pub ambient: Vec3,
    /// Point lights.
    pub points: Vec<PointLight>,
    /// Spot lights.
    pub spots: Vec<SpotLight>,
}

impl LightScene {
    /// Creates a scene from a single directional light.
    pub fn new(directional: DirectionalLight) -> Self {
        Self {
            directional,
            ambient: Vec3::splat(0.02),
            points: Vec::new(),
            spots: Vec::new(),
        }
    }

    /// Builds the legacy uniform (single directional + ambient) for shaders
    /// that have not moved to clustered storage yet.
    pub fn to_legacy_uniform(&self) -> LightUniform {
        let mut uniform = LightUniform::new(&self.directional, self.ambient);
        for light in &self.points {
            uniform.add_point_light(light);
        }
        uniform
    }
}

impl Default for LightScene {
    fn default() -> Self {
        Self::new(DirectionalLight::default())
    }
}
