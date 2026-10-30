//! GPU side of HLOD: uploads a cell proxy as a regular mesh and material, and
//! builds the render object that draws it. CPU logic lives in [`crate::hlod`].

use redox_math::{Aabb, Mat4, Vec3};

use crate::hlod::HlodProxy;
use crate::material::Material;
use crate::mesh::{Mesh, Vertex};
use crate::renderer::Renderer;
use crate::shadow::ShadowClass;
use crate::systems::RenderObject;

/// Mesh and material slots of an uploaded proxy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProxyGpu {
    /// Mesh slot (the `mesh_index` of the render object).
    pub mesh_slot: usize,
    /// Material slot (the `material_index` of the render object).
    pub material_slot: usize,
    /// World-space bounds of the proxy, for culling.
    pub bounds: Aabb,
}

/// A tangent perpendicular to `normal`, for vertices that have no UV-based tangent.
fn perpendicular(normal: [f32; 3]) -> [f32; 3] {
    let n = Vec3::from_array(normal);
    let axis = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    n.cross(axis).normalize_or_zero().to_array()
}

/// The proxy as a CPU mesh (world space, UVs zero).
pub fn proxy_mesh(proxy: &HlodProxy) -> Mesh {
    let vertices = proxy
        .vertices
        .iter()
        .zip(&proxy.normals)
        .map(|(position, normal)| Vertex {
            position: *position,
            normal: *normal,
            uv: [0.0; 2],
            tangent: perpendicular(*normal),
        })
        .collect();
    Mesh::new(vertices, proxy.indices.clone())
}

/// Uploads a proxy mesh and a flat material for its cell. The material colour is the
/// cell's mean colour ([`HlodProxy::mean_colour`]): one colour per cell, variant (b).
pub fn upload_proxy(renderer: &mut Renderer, proxy: &HlodProxy) -> ProxyGpu {
    let mesh = proxy_mesh(proxy);
    let bounds = mesh.aabb;
    let mesh_slot = renderer.upload_mesh(&mesh);
    let material_slot =
        renderer.add_material(Material::solid(Vec3::from_array(proxy.mean_colour())));
    ProxyGpu {
        mesh_slot,
        material_slot,
        bounds,
    }
}

/// Render object that draws an uploaded proxy with the given opacity (the
/// renderer dithers on `color[3]`, see `pbr.wgsl`). `id` must be unique among
/// the shadow casters of the frame.
pub fn proxy_render_object(gpu: &ProxyGpu, id: u32, opacity: f32) -> RenderObject {
    RenderObject {
        id,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::IDENTITY,
        color: [1.0, 1.0, 1.0, opacity],
        mesh_index: gpu.mesh_slot,
        material_index: gpu.material_slot,
        bounds: gpu.bounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mean_colour_is_the_average_of_the_vertex_colours() {
        let proxy = HlodProxy {
            vertices: Vec::new(),
            normals: Vec::new(),
            colors: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            indices: Vec::new(),
            source_triangles: 0,
        };
        let mean = Vec3::from_array(proxy.mean_colour());
        assert!(mean.abs_diff_eq(Vec3::new(0.5, 0.0, 0.5), 1e-6));
    }

    #[test]
    fn a_tangent_is_perpendicular_to_its_normal() {
        for normal in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.3, 0.4, 0.866]] {
            let t = Vec3::from_array(perpendicular(normal));
            assert!(t.dot(Vec3::from_array(normal)).abs() < 1e-5);
            assert!((t.length() - 1.0).abs() < 1e-5);
        }
    }
}
