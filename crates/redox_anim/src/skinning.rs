//! CPU linear-blend skinning over plain vertex arrays (renderer independent).

use redox_math::{Mat4, Vec3};

/// One morph target: per-vertex offsets from the bind pose (glTF `targets[i]`).
/// Arrays are either empty (not animated) or as long as the geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphTarget {
    /// Target name (glTF `targetNames`, or `morph_{i}` when absent).
    pub name: String,
    /// Position offsets per vertex.
    pub positions: Vec<[f32; 3]>,
    /// Normal offsets per vertex. Empty keeps the bind normals.
    pub normals: Vec<[f32; 3]>,
    /// Tangent offsets per vertex. Imported and validated, but not applied by
    /// [`SkinnedGeometry::morph_into`] (morphing only moves positions and normals).
    pub tangents: Vec<[f32; 3]>,
}

/// Bind-pose geometry with per-vertex bone influences (up to four).
#[derive(Clone, Debug, Default)]
pub struct SkinnedGeometry {
    /// Bind-pose vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Bind-pose vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Bind-pose vertex tangents (xyz). Empty means the geometry has no tangents.
    pub tangents: Vec<[f32; 3]>,
    /// Texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Bone indices per vertex.
    pub joints: Vec<[u16; 4]>,
    /// Matching weights per vertex (normalised while deforming).
    pub weights: Vec<[f32; 4]>,
    /// Triangle list indices into the vertex arrays.
    pub indices: Vec<u32>,
    /// Morph targets (blend shapes); empty when the geometry has none.
    pub morph_targets: Vec<MorphTarget>,
    /// Default morph weights (glTF `mesh.weights`); empty means all zero.
    pub morph_defaults: Vec<f32>,
}

impl SkinnedGeometry {
    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Checks that all per-vertex arrays agree in length and indices are in range.
    pub fn validate(&self, bone_count: usize) -> Result<(), String> {
        let n = self.vertex_count();
        if self.normals.len() != n
            || self.uvs.len() != n
            || self.joints.len() != n
            || self.weights.len() != n
        {
            return Err(format!(
                "per-vertex arrays differ in length (positions: {n})"
            ));
        }
        if !self.tangents.is_empty() && self.tangents.len() != n {
            return Err(format!(
                "tangents has {} entries, expected 0 or {n}",
                self.tangents.len()
            ));
        }
        for target in &self.morph_targets {
            let wrong = [&target.positions, &target.normals, &target.tangents]
                .iter()
                .any(|array| !array.is_empty() && array.len() != n);
            if wrong {
                return Err(format!(
                    "morph target '{}' has an array that does not match {n} vertices",
                    target.name
                ));
            }
        }
        if let Some(bad) = self.indices.iter().find(|&&i| i as usize >= n) {
            return Err(format!("index {bad} out of range for {n} vertices"));
        }
        for (v, (joints, weights)) in self.joints.iter().zip(&self.weights).enumerate() {
            for (j, w) in joints.iter().zip(weights) {
                if *w > 0.0 && *j as usize >= bone_count {
                    return Err(format!(
                        "vertex {v} references bone {j}, skeleton has {bone_count}"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Whether the geometry carries tangents (an empty array means none).
    pub fn has_tangents(&self) -> bool {
        !self.tangents.is_empty()
    }

    /// Deforms the tangents by the same blended matrices as `deform_into`.
    /// Output is cleared and refilled; without tangents it stays empty.
    pub fn deform_tangents_into(&self, skin: &[Mat4], out: &mut Vec<[f32; 3]>) {
        out.clear();
        if !self.has_tangents() {
            return;
        }
        for (v, tangent) in self.tangents.iter().enumerate() {
            let blended = blend_matrix(skin, &self.joints[v], &self.weights[v]);
            let t = blended
                .transform_vector3(Vec3::from(*tangent))
                .normalize_or_zero();
            out.push(t.to_array());
        }
    }

    /// Deforms the bind pose by `skin_matrices` (see `Pose::skin_matrices`).
    /// Output vectors are cleared and refilled, so scratch buffers can be reused every frame.
    ///
    /// Morph targets are ignored here; use [`Self::deform_morphed_into`] to apply them.
    pub fn deform_into(
        &self,
        skin_matrices: &[Mat4],
        out_positions: &mut Vec<[f32; 3]>,
        out_normals: &mut Vec<[f32; 3]>,
    ) {
        self.blend_into(
            skin_matrices,
            &self.positions,
            &self.normals,
            out_positions,
            out_normals,
        );
    }

    /// Adds the weighted offsets of all morph targets to the bind pose.
    /// Weights shorter than the target list count as 0, longer ones are ignored, and
    /// non-finite weights (NaN) count as 0. Targets with an empty array contribute nothing to it.
    /// Output vectors are cleared and refilled. Without active targets the output is a copy of
    /// the bind pose; normals are renormalised only when at least one target is active.
    /// Morph tangents are not applied (see [`MorphTarget::tangents`]).
    pub fn morph_into(
        &self,
        weights: &[f32],
        out_positions: &mut Vec<[f32; 3]>,
        out_normals: &mut Vec<[f32; 3]>,
    ) {
        out_positions.clear();
        out_positions.extend_from_slice(&self.positions);
        out_normals.clear();
        out_normals.extend_from_slice(&self.normals);
        let mut any_active = false;
        for (t, target) in self.morph_targets.iter().enumerate() {
            let w = sanitize_weight(weights.get(t).copied().unwrap_or(0.0));
            if w == 0.0 {
                continue;
            }
            any_active = true;
            add_scaled(out_positions, &target.positions, w);
            add_scaled(out_normals, &target.normals, w);
        }
        if any_active {
            for n in out_normals.iter_mut() {
                *n = Vec3::from(*n).normalize_or_zero().to_array();
            }
        }
    }

    /// Skinning on top of morphing: `deform_into`, but the bind pose is first morphed with `weights`.
    /// Without morph targets this is exactly `deform_into`.
    pub fn deform_morphed_into(
        &self,
        skin: &[Mat4],
        weights: &[f32],
        out_positions: &mut Vec<[f32; 3]>,
        out_normals: &mut Vec<[f32; 3]>,
    ) {
        if self.morph_targets.is_empty() {
            self.deform_into(skin, out_positions, out_normals);
            return;
        }
        let (mut positions, mut normals) = (Vec::new(), Vec::new());
        self.morph_into(weights, &mut positions, &mut normals);
        self.blend_into(skin, &positions, &normals, out_positions, out_normals);
    }

    /// Shared skinning loop: blends the bone matrices per vertex and transforms the given
    /// positions and normals (bind pose or morphed pose).
    fn blend_into(
        &self,
        skin: &[Mat4],
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        out_positions: &mut Vec<[f32; 3]>,
        out_normals: &mut Vec<[f32; 3]>,
    ) {
        out_positions.clear();
        out_normals.clear();
        for v in 0..self.vertex_count() {
            let blended = blend_matrix(skin, &self.joints[v], &self.weights[v]);
            let p = blended.transform_point3(Vec3::from(positions[v]));
            let n = blended
                .transform_vector3(Vec3::from(normals[v]))
                .normalize_or_zero();
            out_positions.push(p.to_array());
            out_normals.push(n.to_array());
        }
    }
}

/// Morph weights outside the usable range are treated as 0: NaN and infinite values.
fn sanitize_weight(w: f32) -> f32 {
    if w.is_finite() { w } else { 0.0 }
}

/// Adds `w * offsets` to `out` element-wise; an empty `offsets` array does nothing.
fn add_scaled(out: &mut [[f32; 3]], offsets: &[[f32; 3]], w: f32) {
    for (o, d) in out.iter_mut().zip(offsets) {
        o[0] += w * d[0];
        o[1] += w * d[1];
        o[2] += w * d[2];
    }
}

/// Weighted sum of bone matrices; a vertex without influences stays in the bind pose.
fn blend_matrix(skin: &[Mat4], joints: &[u16; 4], weights: &[f32; 4]) -> Mat4 {
    let mut sum = [0.0f32; 16];
    let mut total = 0.0;
    for (j, w) in joints.iter().zip(weights) {
        let Some(m) = skin.get(*j as usize).filter(|_| *w > 0.0) else {
            continue;
        };
        let cols = m.to_cols_array();
        for (acc, c) in sum.iter_mut().zip(cols) {
            *acc += c * w;
        }
        total += w;
    }
    if total <= f32::EPSILON {
        return Mat4::IDENTITY;
    }
    for acc in &mut sum {
        *acc /= total;
    }
    Mat4::from_cols_array(&sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> SkinnedGeometry {
        SkinnedGeometry {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 2],
            tangents: Vec::new(),
            uvs: vec![[0.0; 2]; 2],
            joints: vec![[0, 0, 0, 0], [0, 1, 0, 0]],
            weights: vec![[1.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]],
            indices: vec![0, 1, 0],
            ..Default::default()
        }
    }

    /// Unit quad in the XY plane, all vertices on bone 0, normals +Z, no tangents.
    fn quad_geometry() -> SkinnedGeometry {
        SkinnedGeometry {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            tangents: Vec::new(),
            uvs: vec![[0.0; 2]; 4],
            joints: vec![[0, 0, 0, 0]; 4],
            weights: vec![[1.0, 0.0, 0.0, 0.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..Default::default()
        }
    }

    #[test]
    fn validate_rejects_a_tangent_array_of_the_wrong_length() {
        let mut g = quad_geometry();
        assert!(g.validate(1).is_ok(), "empty tangents are fine");
        g.tangents = vec![[1.0, 0.0, 0.0]; 3];
        assert!(g.validate(1).is_err());
        g.tangents = vec![[1.0, 0.0, 0.0]; 4];
        assert!(g.validate(1).is_ok());
    }

    #[test]
    fn has_tangents_reflects_the_array() {
        let mut g = quad_geometry();
        assert!(!g.has_tangents());
        g.tangents = vec![[1.0, 0.0, 0.0]; 4];
        assert!(g.has_tangents());
    }

    #[test]
    fn tangents_rotate_with_the_bone_and_stay_unit_length() {
        let mut g = quad_geometry();
        g.tangents = vec![[1.0, 0.0, 0.0]; 4];
        let skin = [Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)];
        let mut out = Vec::new();
        g.deform_tangents_into(&skin, &mut out);
        assert_eq!(out.len(), 4);
        for t in &out {
            assert!((Vec3::from(*t) - Vec3::Y).length() < 1.0e-5);
            assert!((Vec3::from(*t).length() - 1.0).abs() < 1.0e-5);
        }
    }

    #[test]
    fn a_geometry_without_tangents_produces_none() {
        let g = quad_geometry();
        let mut out = vec![[9.0; 3]];
        g.deform_tangents_into(&[Mat4::IDENTITY], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_bone_index_out_of_range_is_ignored_like_in_deform_into() {
        let mut g = quad_geometry();
        g.joints[0] = [5, 0, 0, 0];
        g.weights[0] = [0.5, 0.5, 0.0, 0.0];
        g.tangents = vec![[1.0, 0.0, 0.0]; 4];
        let skin = [Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)];
        let mut out = Vec::new();
        g.deform_tangents_into(&skin, &mut out);
        assert_eq!(out.len(), 4);
        // Bone 5 does not exist: only bone 0 remains and its weight is renormalised to 1.
        assert!((Vec3::from(out[0]) - Vec3::Y).length() < 1.0e-5);
    }

    #[test]
    fn identity_matrices_keep_bind_pose() {
        let g = geometry();
        let (mut p, mut n) = (Vec::new(), Vec::new());
        g.deform_into(&[Mat4::IDENTITY; 2], &mut p, &mut n);
        assert_eq!(p, g.positions);
    }

    #[test]
    fn weights_blend_bone_translations() {
        let g = geometry();
        let skin = [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
        ];
        let (mut p, mut n) = (Vec::new(), Vec::new());
        g.deform_into(&skin, &mut p, &mut n);
        assert_eq!(p[0], [0.0, 0.0, 0.0], "fully on bone 0");
        assert_eq!(p[1], [1.0, 2.0, 0.0], "half moved by bone 1");
    }

    #[test]
    fn rotation_rotates_normals() {
        let g = geometry();
        let rot = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let (mut p, mut n) = (Vec::new(), Vec::new());
        g.deform_into(&[rot, rot], &mut p, &mut n);
        assert!((Vec3::from(n[0]) - Vec3::X).length() < 1.0e-5);
    }

    #[test]
    fn validate_rejects_bad_data() {
        let mut g = geometry();
        assert!(g.validate(2).is_ok());
        assert!(g.validate(1).is_err(), "bone index beyond skeleton");
        g.indices.push(99);
        assert!(g.validate(2).is_err());
    }
}
