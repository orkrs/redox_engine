//! CPU capture of surface cards (GI-02 step 2).
//!
//! Each of an object's six faces is rasterised orthographically along its axis: the nearest
//! surface per texel wins (z-buffer on depth), the albedo is the material colour, the normal is
//! the interpolated world normal, and empty texels are dilated by two texels so bilinear
//! sampling does not see seams. The output is deterministic for the same inputs.
//!
//! Texel encoding (see [`CardImage`]):
//! * `albedo`: `rgb` = material albedo, `a` = emission / [`EMISSION_SCALE`] (0..255);
//! * `normal`: `xyz` = world normal mapped to `[0, 255]`, `a` = depth code, `255` = empty.
//!
//! ASSUMPTION: the card stores a scalar emission only, so an emitter's colour is white. The
//! global SDF keeps a per-voxel colour; cards do not.

use redox_math::{Mat4, Vec2, Vec3};

use super::cards::{CARD_FACES, CardFace, DEPTH_EMPTY, card_size, project_to_card};
use crate::mesh::Mesh;

/// Emission units per alpha step: `a = 255` means `EMISSION_SCALE` radiance.
pub const EMISSION_SCALE: f32 = 16.0;
/// Texels of edge dilation applied after rasterisation.
pub const DILATION_TEXELS: u32 = 2;

/// Surface values of an object, as captured by [`capture_object`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardMaterial {
    /// Base colour times the average texture colour (the caller computes it).
    pub albedo: Vec3,
    /// Emitted radiance; only its largest channel is kept on cards (see module docs).
    pub emissive: Vec3,
}

/// One face of an object: `w * h` texels in row-major order, `x` fastest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardImage {
    /// Width in texels.
    pub w: u32,
    /// Height in texels.
    pub h: u32,
    /// `rgb` albedo, `a` emission scale; zero where empty.
    pub albedo: Vec<[u8; 4]>,
    /// `xyz` normal, `a` depth code (`DEPTH_EMPTY` where empty).
    pub normal: Vec<[u8; 4]>,
}

/// The six card faces of one object.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectCards {
    /// World-space AABB of the captured object (minimum corner).
    pub aabb_min: Vec3,
    /// World-space AABB of the captured object (maximum corner).
    pub aabb_max: Vec3,
    /// Faces in [`CardFace::ALL`] order.
    pub faces: [CardImage; CARD_FACES],
}

/// World-space vertex: position and normal.
#[derive(Clone, Copy)]
struct WorldVertex {
    pos: Vec3,
    normal: Vec3,
}

/// Working buffers of one face while triangles are rasterised.
struct FaceRaster {
    w: u32,
    h: u32,
    depth: Vec<f32>,
    normal: Vec<Vec3>,
    filled: Vec<bool>,
}

impl FaceRaster {
    fn new(w: u32, h: u32) -> Self {
        let n = (w * h) as usize;
        Self {
            w,
            h,
            depth: vec![f32::INFINITY; n],
            normal: vec![Vec3::ZERO; n],
            filled: vec![false; n],
        }
    }
}

/// Captures the six cards of `mesh` placed by `model`.
///
/// Texel size is `texel_world` metres; each card's size comes from [`card_size`].
pub fn capture_object(
    mesh: &Mesh,
    model: &Mat4,
    material: &CardMaterial,
    texel_world: f32,
) -> ObjectCards {
    let verts = world_vertices(mesh, model);
    let (aabb_min, aabb_max) = world_aabb(&verts);
    let faces = CardFace::ALL.map(|face| {
        capture_face(
            face,
            aabb_min,
            aabb_max,
            &verts,
            &mesh.indices,
            material,
            texel_world,
        )
    });
    ObjectCards {
        aabb_min,
        aabb_max,
        faces,
    }
}

fn world_vertices(mesh: &Mesh, model: &Mat4) -> Vec<WorldVertex> {
    // Normals use the inverse transpose, so non-uniform scale keeps them perpendicular.
    let normal_matrix = model.inverse().transpose();
    mesh.vertices
        .iter()
        .map(|v| WorldVertex {
            pos: model.transform_point3(Vec3::from_array(v.position)),
            normal: normal_matrix
                .transform_vector3(Vec3::from_array(v.normal))
                .normalize_or_zero(),
        })
        .collect()
}

fn world_aabb(verts: &[WorldVertex]) -> (Vec3, Vec3) {
    if verts.is_empty() {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    verts.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), v| (lo.min(v.pos), hi.max(v.pos)),
    )
}

fn capture_face(
    face: CardFace,
    aabb_min: Vec3,
    aabb_max: Vec3,
    verts: &[WorldVertex],
    indices: &[u32],
    material: &CardMaterial,
    texel_world: f32,
) -> CardImage {
    let (u, v, _) = face.axes();
    let extent = aabb_max - aabb_min;
    let (w, h) = card_size(extent.to_array()[u], extent.to_array()[v], texel_world);
    let mut raster = FaceRaster::new(w, h);
    for tri in indices.chunks_exact(3) {
        let corners = [tri[0], tri[1], tri[2]].map(|i| verts[i as usize]);
        rasterise_triangle(face, aabb_min, aabb_max, &corners, &mut raster);
    }
    let mut image = encode(&raster, face, material);
    for _ in 0..DILATION_TEXELS {
        dilate_once(&mut image);
    }
    image
}

/// Z-buffered rasterisation of one triangle: a texel is covered when its centre is inside.
fn rasterise_triangle(
    face: CardFace,
    aabb_min: Vec3,
    aabb_max: Vec3,
    corners: &[WorldVertex; 3],
    raster: &mut FaceRaster,
) {
    let mut screen = [Vec2::ZERO; 3];
    let mut depth = [0.0f32; 3];
    for (k, c) in corners.iter().enumerate() {
        let (uv, d) = project_to_card(face, aabb_min, aabb_max, c.pos);
        screen[k] = Vec2::new(uv.x * raster.w as f32, uv.y * raster.h as f32);
        depth[k] = d;
    }
    let area = edge(screen[0], screen[1], screen[2]);
    if area.abs() < 1e-12 {
        return;
    }
    let (min, max) = screen.iter().fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(lo, hi), s| (lo.min(*s), hi.max(*s)),
    );
    let x0 = min.x.floor().max(0.0) as u32;
    let y0 = min.y.floor().max(0.0) as u32;
    let x1 = (max.x.ceil().max(0.0) as u32).min(raster.w.saturating_sub(1));
    let y1 = (max.y.ceil().max(0.0) as u32).min(raster.h.saturating_sub(1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let b0 = edge(screen[1], screen[2], c) / area;
            let b1 = edge(screen[2], screen[0], c) / area;
            let b2 = 1.0 - b0 - b1;
            if b0 < -1e-6 || b1 < -1e-6 || b2 < -1e-6 {
                continue;
            }
            let d = b0 * depth[0] + b1 * depth[1] + b2 * depth[2];
            let i = (x + y * raster.w) as usize;
            if d < raster.depth[i] {
                raster.depth[i] = d;
                raster.filled[i] = true;
                let n = corners[0].normal * b0 + corners[1].normal * b1 + corners[2].normal * b2;
                raster.normal[i] = n.normalize_or_zero();
            }
        }
    }
}

fn edge(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (c.x - a.x) * (b.y - a.y) - (c.y - a.y) * (b.x - a.x)
}

/// Encodes the raster into texels. Empty texels get zero albedo and the empty depth code.
fn encode(raster: &FaceRaster, _face: CardFace, material: &CardMaterial) -> CardImage {
    let emission = (material.emissive.max_element() / EMISSION_SCALE).clamp(0.0, 1.0);
    let albedo_texel = [
        unit_to_u8(material.albedo.x),
        unit_to_u8(material.albedo.y),
        unit_to_u8(material.albedo.z),
        unit_to_u8(emission),
    ];
    let n = raster.filled.len();
    let mut albedo = vec![[0u8; 4]; n];
    let mut normal = vec![[0u8, 0, 0, DEPTH_EMPTY]; n];
    for i in 0..n {
        if !raster.filled[i] {
            continue;
        }
        albedo[i] = albedo_texel;
        let nn = raster.normal[i];
        let depth_code = (raster.depth[i].clamp(0.0, 1.0) * 254.0).round() as u8;
        normal[i] = [
            unit_to_u8(nn.x * 0.5 + 0.5),
            unit_to_u8(nn.y * 0.5 + 0.5),
            unit_to_u8(nn.z * 0.5 + 0.5),
            depth_code,
        ];
    }
    CardImage {
        w: raster.w,
        h: raster.h,
        albedo,
        normal,
    }
}

fn unit_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn is_empty(normal: &[u8; 4]) -> bool {
    normal[3] == DEPTH_EMPTY
}

/// One dilation pass: every empty texel with a filled 8-neighbour copies the first such
/// neighbour (in a fixed scan order), reading from a snapshot so the pass is order independent.
fn dilate_once(image: &mut CardImage) {
    let src = image.clone();
    let (w, h) = (src.w as i64, src.h as i64);
    for y in 0..h {
        for x in 0..w {
            let i = (x + y * w) as usize;
            if !is_empty(&src.normal[i]) {
                continue;
            }
            let found = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .filter(|&(dx, dy)| (dx, dy) != (0, 0))
                .map(|(dx, dy)| (x + dx, y + dy))
                .find(|&(nx, ny)| {
                    nx >= 0
                        && ny >= 0
                        && nx < w
                        && ny < h
                        && !is_empty(&src.normal[(nx + ny * w) as usize])
                });
            if let Some((nx, ny)) = found {
                let j = (nx + ny * w) as usize;
                image.albedo[i] = src.albedo[j];
                image.normal[i] = src.normal[j];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Vertex;

    fn vertex(position: [f32; 3], normal: [f32; 3]) -> Vertex {
        Vertex {
            position,
            normal,
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
        }
    }

    /// Axis-aligned cube from `-half` to `half`, 12 triangles, outward normals.
    fn cube(half: f32) -> Mesh {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let mut n = [0.0f32; 3];
                n[axis] = sign;
                let corner = |su: f32, sv: f32| {
                    let mut p = [0.0f32; 3];
                    p[axis] = sign * half;
                    p[u] = su * half;
                    p[v] = sv * half;
                    vertex(p, n)
                };
                let base = vertices.len() as u32;
                vertices.extend([
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ]);
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        Mesh::new(vertices, indices)
    }

    /// UV sphere of radius `r` centred at the origin.
    fn sphere(r: f32, slices: u32, stacks: u32) -> Mesh {
        let mut vertices = Vec::new();
        for s in 0..=stacks {
            let phi = std::f32::consts::PI * s as f32 / stacks as f32;
            for k in 0..=slices {
                let theta = std::f32::consts::TAU * k as f32 / slices as f32;
                let n = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
                vertices.push(vertex([n[0] * r, n[1] * r, n[2] * r], n));
            }
        }
        let mut indices = Vec::new();
        let row = slices + 1;
        for s in 0..stacks {
            for k in 0..slices {
                let a = s * row + k;
                let b = a + row;
                indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        Mesh::new(vertices, indices)
    }

    fn red() -> CardMaterial {
        CardMaterial {
            albedo: Vec3::new(1.0, 0.0, 0.0),
            emissive: Vec3::ZERO,
        }
    }

    #[test]
    fn a_cube_card_is_filled_with_the_material_colour() {
        let cards = capture_object(&cube(1.0), &Mat4::IDENTITY, &red(), 0.25);
        let face = &cards.faces[CardFace::PosX.index()];
        assert!(face.normal.iter().all(|n| n[3] != DEPTH_EMPTY));
        assert!(face.albedo.iter().all(|a| *a == [255, 0, 0, 0]));
    }

    #[test]
    fn the_normal_of_a_cube_face_points_outward() {
        let cards = capture_object(&cube(1.0), &Mat4::IDENTITY, &red(), 0.25);
        let face = &cards.faces[CardFace::PosX.index()];
        let n = face.normal[(face.w * face.h / 2) as usize];
        assert!(n[0] >= 254 && (n[1] as i32 - 128).abs() <= 1 && (n[2] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn a_sphere_card_has_empty_corners_and_a_filled_centre() {
        let cards = capture_object(&sphere(1.0, 32, 16), &Mat4::IDENTITY, &red(), 0.1);
        let face = &cards.faces[CardFace::PosZ.index()];
        // Dilation is two texels, so only the far corner beyond the disc stays empty.
        let corner = face.normal[0];
        assert_eq!(corner[3], DEPTH_EMPTY);
        let mid = (face.w * face.h / 2 + face.w / 2) as usize;
        assert!(
            face.normal[mid][3] < 128,
            "centre depth {}",
            face.normal[mid][3]
        );
    }

    #[test]
    fn the_depth_of_a_tilted_plane_grows_linearly() {
        // Triangle in the plane y = x, seen along -Y from above (PosY face).
        let verts = vec![
            vertex([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            vertex([2.0, 2.0, 0.0], [0.0, 1.0, 0.0]),
            vertex([0.0, 0.0, 2.0], [0.0, 1.0, 0.0]),
        ];
        let mesh = Mesh::new(verts, vec![0, 1, 2]);
        let cards = capture_object(&mesh, &Mat4::IDENTITY, &red(), 0.25);
        let face = &cards.faces[CardFace::PosY.index()];
        // Along u (= x) the surface rises (y = x) towards the camera above, so the depth
        // (= (2 - y) / 2) shrinks linearly with u.
        let row = face.h / 2;
        let mut last = i32::MAX;
        let mut seen = 0;
        for x in 0..face.w {
            let code = face.normal[(x + row * face.w) as usize][3];
            if code == DEPTH_EMPTY {
                continue;
            }
            assert!(code as i32 <= last, "depth grew at x={x}");
            last = code as i32;
            seen += 1;
        }
        assert!(seen > 2 && last < i32::MAX);
    }

    #[test]
    fn dilation_fills_one_texel_gaps() {
        let mut image = CardImage {
            w: 3,
            h: 3,
            albedo: vec![[9, 9, 9, 0]; 9],
            normal: vec![[1, 2, 3, 10]; 9],
        };
        image.normal[4] = [0, 0, 0, DEPTH_EMPTY];
        image.albedo[4] = [0; 4];
        dilate_once(&mut image);
        assert_eq!(image.normal[4][3], 10);
        assert_eq!(image.albedo[4], [9, 9, 9, 0]);
    }

    #[test]
    fn the_capture_is_deterministic() {
        let a = capture_object(&sphere(1.0, 24, 12), &Mat4::IDENTITY, &red(), 0.1);
        let b = capture_object(&sphere(1.0, 24, 12), &Mat4::IDENTITY, &red(), 0.1);
        assert_eq!(a, b);
    }
}
