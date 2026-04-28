use redox_math::{Mat4, Vec2, Vec3, Vec4, orthographic, look_at};

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
    pub _pad: [u32; 3],
}

unsafe impl bytemuck::Pod for ShadowUniform {}
unsafe impl bytemuck::Zeroable for ShadowUniform {}

impl Default for ShadowUniform {
    fn default() -> Self {
        Self {
            csm_matrices: [[[0.0; 4]; 4]; CSM_CASCADES],
            csm_splits: [0.0; CSM_CASCADES],
            shadow_atlas_size: 4096.0,
            _pad: [0; 3],
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
    /// `view` / `proj` — camera matrices; `light_dir` must be normalised and point FROM
    /// the scene TOWARDS the light (sun direction).
    pub fn update(
        &mut self,
        view: Mat4,
        proj: Mat4,
        light_dir: Vec3,
        shadow_map_resolution: u32,
    ) -> ShadowUniform {
        let cfg = self.config;
        let cascade_count = cfg.cascade_count.min(CSM_CASCADES).max(1);

        // Compute inverse view-projection for frustum corner reconstruction.
        let vp = proj * view;
        let inv_vp = vp.inverse();

        // Logarithmic split scheme.
        let n = cfg.near;
        let f = cfg.far;
        for i in 0..cascade_count {
            let si = (i + 1) as f32 / cascade_count as f32;
            let log = n * (f / n).powf(si);
            self.splits[i] = log;
        }
        for i in cascade_count..CSM_CASCADES {
            self.splits[i] = f;
        }

        // For each cascade build tight light-space AABB and corresponding ortho+view.
        for ci in 0..cascade_count {
            let prev_split = if ci == 0 { n } else { self.splits[ci - 1] };
            let curr_split = self.splits[ci];

            // 8 corners in clip space for this cascade slice (view-space z in [prev_split, curr_split]).
            let mut frustum_corners = [Vec3::ZERO; 8];
            let mut idx = 0;
            for &z in &[prev_split, curr_split] {
                for &x in &[-1.0_f32, 1.0] {
                    for &y in &[-1.0_f32, 1.0] {
                        // Reconstruct view-space position on near/far slice by projecting clip-space
                        // corners through inverse VP. For simplicity we sample at z normalized
                        // between near/far using linear depth; this is sufficient for stable cascades.
                        let clip = Vec4::new(x, y, 1.0, 1.0);
                        let world = inv_vp * clip;
                        let wp = world.truncate() / world.w;
                        let view_pos = (view * Vec4::from((wp, 1.0))).truncate();
                        let dir = view_pos.normalize();
                        let p = dir * z;
                        frustum_corners[idx] = Vec3::new(p.x, p.y, p.z);
                        idx += 1;
                    }
                }
            }

            // Transform frustum corners to world space.
            let inv_view = view.inverse();
            for c in &mut frustum_corners {
                let v = Vec4::new(c.x, c.y, c.z, 1.0);
                let w = inv_view * v;
                *c = w.truncate();
            }

            // Build bounding sphere for this slice.
            let mut center = Vec3::ZERO;
            for c in &frustum_corners {
                center += *c;
            }
            center /= frustum_corners.len() as f32;
            let mut radius = 0.0_f32;
            for c in &frustum_corners {
                radius = radius.max((*c - center).length());
            }

            // Build light-space orthographic projection around the sphere.
            let light_dir_n = light_dir.normalize();
            let eye = center - light_dir_n * radius * 2.0;
            let target = center;
            let up = Vec3::Y;
            let view_l = look_at(eye, target, up);

            let left = -radius;
            let right = radius;
            let bottom = -radius;
            let top = radius;
            // Push near/far slightly beyond sphere to be safe.
            let near_l = 0.0_f32;
            let far_l = radius * 4.0;
            let proj_l = orthographic(left, right, bottom, top, near_l, far_l);

            let mut light_vp = proj_l * view_l;

            // Stabilise by snapping to texel grid in light space.
            let shadow_res = shadow_map_resolution as f32;
            let texel_size = (right - left) / shadow_res;
            // Project origin to light clip space and snap xy.
            let origin = Mat4::IDENTITY;
            let origin_ls = light_vp * origin;
            let offset = Vec2::new(origin_ls.w_axis.x, origin_ls.w_axis.y) / origin_ls.w_axis.w;
            let offset_world = Vec2::new(
                (offset.x / texel_size).round() * texel_size - offset.x,
                (offset.y / texel_size).round() * texel_size - offset.y,
            );
            let correction = Mat4::from_translation(Vec3::new(offset_world.x, offset_world.y, 0.0));
            light_vp = proj_l * view_l * correction;

            self.cascades[ci] = light_vp;
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
        u
    }
}

