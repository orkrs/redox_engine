use redox_math::{Mat4, Vec3, look_at, orthographic};

/// Maximum number of cascades for directional light shadows.
pub const CSM_CASCADES: usize = 4;

/// GPU-side shadow uniform matching WGSL layout.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ShadowUniform {
    // [cascade][row][col]
    pub csm_matrices: [[[f32; 4]; 4]; CSM_CASCADES],
    pub csm_splits: [f32; CSM_CASCADES],
    pub shadow_atlas_size: f32,
    /// Near plane used for the CSM config (PCSS linearization).
    pub soft_near: f32,
    /// Far plane used for the CSM config (PCSS linearization).
    pub soft_far: f32,
    pub _pad2: u32,
}

unsafe impl bytemuck::Pod for ShadowUniform {}
unsafe impl bytemuck::Zeroable for ShadowUniform {}

impl Default for ShadowUniform {
    fn default() -> Self {
        Self {
            csm_matrices: [[[0.0; 4]; 4]; CSM_CASCADES],
            csm_splits: [0.0; CSM_CASCADES],
            shadow_atlas_size: 2048.0,
            soft_near: 0.1,
            soft_far: 200.0,
            _pad2: 0,
        }
    }
}

/// CPU-side CSM configuration.
#[derive(Clone, Copy, Debug)]
pub struct CsmConfig {
    pub cascade_count: usize,
    pub shadow_map_resolution: u32,
    pub near: f32,
    pub far: f32,
}

impl Default for CsmConfig {
    fn default() -> Self {
        Self {
            cascade_count: CSM_CASCADES,
            shadow_map_resolution: 2048,
            near: 0.1,
            far: 200.0,
        }
    }
}

/// CPU-side CSM state: computes cascade matrices each frame.
#[derive(Debug)]
pub struct CsmState {
    pub config: CsmConfig,
    pub cascades: [Mat4; CSM_CASCADES],
    pub splits: [f32; CSM_CASCADES],
}

impl CsmState {
    pub fn new(config: CsmConfig) -> Self {
        Self {
            config,
            cascades: [Mat4::IDENTITY; CSM_CASCADES],
            splits: [0.0; CSM_CASCADES],
        }
    }

    /// Recomputes cascade view-projection matrices for the given camera and light.
    ///
    /// `view` / `proj` — camera matrices; `cam_near` / `cam_far` **must** match the
    /// planes baked into `proj` (mismatched far is the classic “shadow chunks on
    /// the ground” bug). `light_dir` is normalised and points FROM the scene
    /// TOWARDS the light (sun direction).
    ///
    /// Per cascade:
    /// 1. Reconstruct the slice's world-space corners by unprojecting NDC corners
    ///    at the cascade's depth range ([0, 1] clip convention).
    /// 2. Fit a bounding sphere.
    /// 3. Build a light-space view looking at the sphere center.
    /// 4. Snap that center to a texel grid in light-view space (kills swimming).
    pub fn update(
        &mut self,
        view: Mat4,
        proj: Mat4,
        light_dir: Vec3,
        shadow_map_resolution: u32,
        cam_near: f32,
        cam_far: f32,
    ) -> ShadowUniform {
        // Keep config in sync so callers / soft_* uniforms stay consistent.
        self.config.near = cam_near.max(1.0e-3);
        self.config.far = cam_far.max(self.config.near + 1.0);
        let cfg = self.config;
        let cascade_count = cfg.cascade_count.min(CSM_CASCADES).max(1);

        // Logarithmic split scheme over the *actual* camera range.
        let n = cfg.near;
        let f = cfg.far;
        for i in 0..cascade_count {
            let si = (i + 1) as f32 / cascade_count as f32;
            self.splits[i] = n * (f / n).powf(si);
        }
        for i in cascade_count..CSM_CASCADES {
            self.splits[i] = f;
        }

        let inv_view = view.inverse();
        // Frustum half-extents from the projection (avoids NDC depth mismatch bugs).
        let p = proj.to_cols_array_2d();
        let tan_half_x = 1.0 / p[0][0].abs().max(1e-6);
        let tan_half_y = 1.0 / p[1][1].abs().max(1e-6);

        let light_dir_n = light_dir.normalize();
        // Stable up vector for the light view.
        let up = if light_dir_n.dot(Vec3::Y).abs() > 0.99 {
            Vec3::X
        } else {
            Vec3::Y
        };

        for ci in 0..cascade_count {
            let prev_split = if ci == 0 { n } else { self.splits[ci - 1] };
            let curr_split = self.splits[ci];

            // ── 1. World-space corners of the view-frustum slice ──
            let mut min_w = Vec3::splat(f32::INFINITY);
            let mut max_w = Vec3::splat(f32::NEG_INFINITY);
            for &depth in &[prev_split, curr_split] {
                for &sx in &[-1.0_f32, 1.0] {
                    for &sy in &[-1.0_f32, 1.0] {
                        // RH view space: camera looks down −Z, depth = −z.
                        let view_pos = Vec3::new(
                            sx * tan_half_x * depth,
                            sy * tan_half_y * depth,
                            -depth,
                        );
                        let world = inv_view.transform_point3(view_pos);
                        min_w = min_w.min(world);
                        max_w = max_w.max(world);
                    }
                }
            }
            if !min_w.is_finite() || !max_w.is_finite() {
                self.cascades[ci] = Mat4::IDENTITY;
                continue;
            }

            // ── 2. Bounding sphere ──
            let center = (min_w + max_w) * 0.5;
            let radius = ((max_w - min_w).length() * 0.5).max(0.001);

            // ── 3. Light view around the slice center ──
            // Eye sits ON the light side so look_at aims along −light_dir.
            let eye_dist = radius * 2.5;
            let base_eye = center + light_dir_n * eye_dist;
            let light_view = look_at(base_eye, center, up);

            // ── 4. Texel snap in light-view space (kills cascade swimming) ──
            let texel = (2.0 * radius) / shadow_map_resolution as f32;
            let inv_light = light_view.inverse();
            let ls_center = light_view.transform_point3(center);
            let snapped_ls = Vec3::new(
                (ls_center.x / texel).round() * texel,
                (ls_center.y / texel).round() * texel,
                ls_center.z,
            );
            let snapped_world = inv_light.transform_point3(snapped_ls);
            let snapped_eye = snapped_world + light_dir_n * eye_dist;
            let snapped_view = look_at(snapped_eye, snapped_world, up);

            // Fit near/far tightly around the cascade sphere. Using near≈0 and
            // far=6R wastes ~40%+ of the Depth32 range on empty space in front
            // of the sphere — cast shadows vanish (receiver vs occluder delta
            // collapses into a few ulps, PCF washes them out).
            let near_l = (eye_dist - radius * 1.05).max(0.01);
            let far_l = eye_dist + radius * 1.05;
            let proj_l = orthographic(-radius, radius, -radius, radius, near_l, far_l);

            self.cascades[ci] = proj_l * snapped_view;
        }

        // Pack into GPU uniform.
        let mut u = ShadowUniform::default();
        for ci in 0..CSM_CASCADES {
            let mat = if ci < cascade_count {
                self.cascades[ci]
            } else {
                Mat4::IDENTITY
            };
            u.csm_matrices[ci] = mat.to_cols_array_2d();
        }
        u.csm_splits = self.splits;
        u.shadow_atlas_size = shadow_map_resolution as f32;
        u.soft_near = n;
        u.soft_far = f;
        u
    }
}
