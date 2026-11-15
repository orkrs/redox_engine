use redox_math::Vec3;

use crate::light::LightScene;
use crate::mesh::Mesh;

use super::uv::{DEFAULT_METERS_PER_TEXEL, MeshUv2Error, reject_mesh_without_uv2};

pub const BAKE_RAYS: u32 = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct LightmapImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LightmapBakeError {
    NoUv2,
    EmptyMesh,
}

impl From<MeshUv2Error> for LightmapBakeError {
    fn from(value: MeshUv2Error) -> Self {
        match value {
            MeshUv2Error::MissingUv2 => Self::NoUv2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LightmapScene<'a> {
    pub meshes: &'a [Mesh],
    pub lights: &'a LightScene,
}

#[derive(Clone, Debug)]
pub struct LightmapBaker {
    pub seed: u64,
    pub meters_per_texel: f32,
}

impl Default for LightmapBaker {
    fn default() -> Self {
        Self {
            seed: 1,
            meters_per_texel: DEFAULT_METERS_PER_TEXEL,
        }
    }
}

impl LightmapBaker {
    pub fn bake_mesh(
        &self,
        mesh: &Mesh,
        scene: &LightmapScene<'_>,
        width: u32,
        height: u32,
    ) -> Result<LightmapImage, LightmapBakeError> {
        reject_mesh_without_uv2(mesh)?;
        if mesh.vertices.is_empty() {
            return Err(LightmapBakeError::EmptyMesh);
        }
        let mut pixels = vec![0u16; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let uv = [
                    (x as f32 + 0.5) / width as f32,
                    (y as f32 + 0.5) / height as f32,
                ];
                let hit = sample(mesh, uv);
                let (r, g, b, a) = if let Some(hit) = hit {
                    let to_light = scene.lights.directional.direction.normalize_or_zero();
                    let mut ndotl = hit.normal.dot(to_light).max(0.0);
                    let origin = hit.position + hit.normal * 0.05;
                    if ndotl > 0.0 && scene_blocks(origin, to_light, scene.meshes) {
                        ndotl = 0.0;
                    }
                    (
                        ndotl,
                        ndotl * 0.9,
                        ndotl * 0.8,
                        if ndotl > 0.0 { 1.0 } else { 0.0 },
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                };
                let i = ((y * width + x) * 4) as usize;
                for (c, v) in [r, g, b, a].into_iter().enumerate() {
                    pixels[i + c] = half::f16::from_f32(v).to_bits();
                }
            }
        }
        Ok(LightmapImage {
            width,
            height,
            pixels,
        })
    }
}

struct SurfaceHit {
    position: Vec3,
    normal: Vec3,
}

fn sample(mesh: &Mesh, uv: [f32; 2]) -> Option<SurfaceHit> {
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (
            &mesh.vertices[tri[0] as usize],
            &mesh.vertices[tri[1] as usize],
            &mesh.vertices[tri[2] as usize],
        );
        if let Some((w0, w1, w2)) = bary(a.uv2, b.uv2, c.uv2, uv) {
            let n = Vec3::new(
                a.normal[0] * w0 + b.normal[0] * w1 + c.normal[0] * w2,
                a.normal[1] * w0 + b.normal[1] * w1 + c.normal[1] * w2,
                a.normal[2] * w0 + b.normal[2] * w1 + c.normal[2] * w2,
            )
            .normalize_or_zero();
            let position = Vec3::from_array(a.position) * w0
                + Vec3::from_array(b.position) * w1
                + Vec3::from_array(c.position) * w2;
            return Some(SurfaceHit {
                position,
                normal: n,
            });
        }
    }
    None
}

fn scene_blocks(origin: Vec3, to_light: Vec3, meshes: &[Mesh]) -> bool {
    meshes
        .iter()
        .any(|mesh| ray_hits_mesh(origin, to_light, mesh))
}

fn ray_hits_mesh(origin: Vec3, dir: Vec3, mesh: &Mesh) -> bool {
    mesh.indices.chunks_exact(3).any(|tri| {
        let a = Vec3::from_array(mesh.vertices[tri[0] as usize].position);
        let b = Vec3::from_array(mesh.vertices[tri[1] as usize].position);
        let c = Vec3::from_array(mesh.vertices[tri[2] as usize].position);
        ray_hits_triangle(origin, dir, a, b, c)
    })
}

fn ray_hits_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> bool {
    let edge1 = b - a;
    let edge2 = c - a;
    let p = dir.cross(edge2);
    let det = edge1.dot(p);
    if det.abs() < 1e-6 {
        return false;
    }
    let inv = 1.0 / det;
    let tvec = origin - a;
    let u = tvec.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = tvec.cross(edge1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = edge2.dot(q) * inv;
    t > 1e-4
}

fn bary(a: [f32; 2], b: [f32; 2], c: [f32; 2], p: [f32; 2]) -> Option<(f32, f32, f32)> {
    let v0 = [b[0] - a[0], b[1] - a[1]];
    let v1 = [c[0] - a[0], c[1] - a[1]];
    let v2 = [p[0] - a[0], p[1] - a[1]];
    let den = v0[0] * v1[1] - v1[0] * v0[1];
    if den.abs() < 1e-8 {
        return None;
    }
    let w1 = (v2[0] * v1[1] - v1[0] * v2[1]) / den;
    let w2 = (v0[0] * v2[1] - v2[0] * v0[1]) / den;
    let w0 = 1.0 - w1 - w2;
    (w0 >= -1e-3 && w1 >= -1e-3 && w2 >= -1e-3).then_some((w0, w1, w2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::DirectionalLight;
    use crate::mesh::Vertex;

    #[test]
    fn a_texel_facing_the_sun_is_brighter_than_a_texel_facing_away() {
        let mesh = Mesh::new(
            vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                    uv2: [0.0, 0.0],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [1.0, 0.0],
                    uv2: [1.0, 0.0],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [0.0, 0.0, 1.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 1.0],
                    uv2: [0.0, 1.0],
                    tangent: [1.0, 0.0, 0.0],
                },
            ],
            vec![0, 1, 2],
        );
        let scene = LightmapScene {
            meshes: &[mesh.clone()],
            lights: &LightScene::new(DirectionalLight::new(
                Vec3::new(0.1, 1.0, 0.0),
                Vec3::ONE,
                2.0,
            )),
        };
        let img = LightmapBaker::default()
            .bake_mesh(&mesh, &scene, 4, 4)
            .unwrap();
        assert!(
            img.pixels
                .iter()
                .any(|&h| half::f16::from_bits(h).to_f32() > 0.1)
        );
    }

    #[test]
    fn a_texel_behind_a_wall_is_dark() {
        let floor = Mesh::new(
            vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                    uv2: [0.0, 0.0],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [1.0, 0.0],
                    uv2: [1.0, 0.0],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [0.0, 0.0, 1.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 1.0],
                    uv2: [0.0, 1.0],
                    tangent: [1.0, 0.0, 0.0],
                },
            ],
            vec![0, 1, 2],
        );
        let wall = Mesh::new(
            vec![
                Vertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, -1.0, 0.0],
                    uv: [0.0, 0.0],
                    uv2: [0.0, 0.0],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 1.0, 0.0],
                    normal: [0.0, -1.0, 0.0],
                    uv: [1.0, 0.0],
                    uv2: [1.0, 0.0],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [0.0, 1.0, 1.0],
                    normal: [0.0, -1.0, 0.0],
                    uv: [0.0, 1.0],
                    uv2: [0.0, 1.0],
                    tangent: [1.0, 0.0, 0.0],
                },
            ],
            vec![0, 1, 2],
        );
        let lights = LightScene::new(DirectionalLight::new(Vec3::Y, Vec3::ONE, 1.0));
        let open = LightmapScene {
            meshes: &[],
            lights: &lights,
        };
        let blocked = LightmapScene {
            meshes: std::slice::from_ref(&wall),
            lights: &lights,
        };
        let baker = LightmapBaker::default();
        let lit = baker.bake_mesh(&floor, &open, 2, 2).unwrap();
        let dark = baker.bake_mesh(&floor, &blocked, 2, 2).unwrap();
        assert!(
            lit.pixels
                .iter()
                .any(|&h| half::f16::from_bits(h).to_f32() > 0.1)
        );
        assert!(
            dark.pixels
                .iter()
                .all(|&h| half::f16::from_bits(h).to_f32() < 0.02)
        );
    }

    #[test]
    fn padding_texels_have_zero_occupancy() {
        let mesh = Mesh::new(
            vec![Vertex {
                position: [0.0; 3],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
                uv2: [0.9, 0.9],
                tangent: [1.0, 0.0, 0.0],
            }],
            vec![0, 0, 0],
        );
        let scene = LightmapScene {
            meshes: &[],
            lights: &LightScene::new(DirectionalLight::new(Vec3::Y, Vec3::ONE, 0.0)),
        };
        let img = LightmapBaker::default()
            .bake_mesh(&mesh, &scene, 2, 2)
            .unwrap();
        assert_eq!(half::f16::from_bits(img.pixels[3]).to_f32(), 0.0);
    }

    #[test]
    fn baking_twice_matches_within_1_percent() {
        let mesh = Mesh::new(
            vec![
                Vertex {
                    position: [0.0; 3],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                    uv2: [0.2, 0.2],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                    uv2: [0.8, 0.2],
                    tangent: [1.0, 0.0, 0.0],
                },
                Vertex {
                    position: [0.0, 0.0, 1.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                    uv2: [0.2, 0.8],
                    tangent: [1.0, 0.0, 0.0],
                },
            ],
            vec![0, 1, 2],
        );
        let scene = LightmapScene {
            meshes: &[mesh.clone()],
            lights: &LightScene::new(DirectionalLight::new(
                Vec3::new(0.2, -1.0, 0.0),
                Vec3::ONE,
                1.0,
            )),
        };
        let baker = LightmapBaker::default();
        let a = baker.bake_mesh(&mesh, &scene, 8, 8).unwrap();
        let b = baker.bake_mesh(&mesh, &scene, 8, 8).unwrap();
        assert_eq!(a.pixels, b.pixels);
    }
}
