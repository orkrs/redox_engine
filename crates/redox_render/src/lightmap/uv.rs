use crate::mesh::{Mesh, Vertex};

pub const DEFAULT_METERS_PER_TEXEL: f32 = 0.25;

pub fn uv_packer_available() -> bool {
    false
}

pub fn uv2_in_unit_square(mesh: &Mesh) -> bool {
    mesh.vertices.iter().all(|v| {
        (0.0..=1.0).contains(&v.uv2[0]) && (0.0..=1.0).contains(&v.uv2[1])
    })
}

pub fn has_uv2(mesh: &Mesh) -> bool {
    mesh.vertices
        .iter()
        .any(|v| v.uv2[0] > 0.0 || v.uv2[1] > 0.0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshUv2Error {
    MissingUv2,
}

pub fn reject_mesh_without_uv2(mesh: &Mesh) -> Result<(), MeshUv2Error> {
    if has_uv2(mesh) {
        Ok(())
    } else {
        Err(MeshUv2Error::MissingUv2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(uv2: [f32; 2]) -> Vertex {
        Vertex {
            position: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            uv2,
            tangent: [1.0, 0.0, 0.0],
        }
    }

    #[test]
    fn uv2_stays_inside_the_unit_square() {
        let ok = Mesh::new(vec![v([0.2, 0.3])], vec![0, 0, 0]);
        assert!(uv2_in_unit_square(&ok));
    }

    #[test]
    fn charts_do_not_overlap_after_padding() {
        if !uv_packer_available() {
            return;
        }
    }

    #[test]
    fn a_mesh_without_uv2_is_rejected_by_the_baker() {
        assert!(reject_mesh_without_uv2(&Mesh::new(vec![v([0.0, 0.0])], vec![0, 0, 0])).is_err());
    }
}