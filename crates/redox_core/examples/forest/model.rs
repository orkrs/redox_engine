//! Loads the `.glb` models of the forest (exported from Blender) into renderer meshes and
//! materials: base colour, normal and metallic-roughness textures, masked leaves.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use redox_math::Vec3;
use redox_render::Renderer;
use redox_render::material::Material;
use redox_render::mesh::{Mesh, Vertex};

/// One drawable piece of a model: a mesh slot and the material slot it is drawn with.
#[derive(Clone, Copy)]
pub struct Part {
    pub mesh: usize,
    pub material: usize,
}

/// A loaded model: its parts share one local frame (origin at the foot, Y up).
pub struct Model {
    pub parts: Vec<Part>,
}

/// Texture slots already uploaded, by image name (the same image appears in several files).
#[derive(Default)]
pub struct TextureCache {
    srgb: HashMap<String, usize>,
    linear: HashMap<String, usize>,
}

fn encode_png(image: &gltf::image::Data, hard_alpha: bool) -> Result<Vec<u8>, String> {
    use gltf::image::Format as F;
    // (channels, bytes per channel); 16-bit channels keep their high byte.
    let (channels, bytes) = match image.format {
        F::R8 => (1, 1),
        F::R8G8 => (2, 1),
        F::R8G8B8 => (3, 1),
        F::R8G8B8A8 => (4, 1),
        F::R16 => (1, 2),
        F::R16G16 => (2, 2),
        F::R16G16B16 => (3, 2),
        F::R16G16B16A16 => (4, 2),
        other => return Err(format!("unsupported texture format {other:?}")),
    };
    let rgba: Vec<u8> = image
        .pixels
        .chunks_exact(channels * bytes)
        .flat_map(|px| {
            let c = |i: usize| px[i * bytes + bytes - 1];
            match channels {
                1 => [c(0), c(0), c(0), 255],
                2 => [c(0), c(1), 0, 255],
                3 => [c(0), c(1), c(2), 255],
                _ if hard_alpha => [c(0), c(1), c(2), if c(3) >= 128 { 255 } else { 0 }],
                _ => [c(0), c(1), c(2), c(3)],
            }
        })
        .collect();
    let buffer = image::RgbaImage::from_raw(image.width, image.height, rgba)
        .ok_or("texture pixels do not match its size")?;
    let mut out = Cursor::new(Vec::new());
    buffer
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

fn upload(
    r: &mut Renderer,
    cache: &mut TextureCache,
    document: &gltf::Document,
    images: &[gltf::image::Data],
    texture: gltf::Texture<'_>,
    srgb: bool,
    hard_alpha: bool,
) -> Result<usize, String> {
    let source = texture.source();
    let index = source.index();
    let name = source
        .name()
        .map_or_else(|| format!("image_{index}"), str::to_owned)
        + if hard_alpha { "#mask" } else { "" };
    let table = if srgb {
        &mut cache.srgb
    } else {
        &mut cache.linear
    };
    if let Some(&slot) = table.get(&name) {
        return Ok(slot);
    }
    let _ = document;
    let png = encode_png(&images[index], hard_alpha)?;
    let slot = if srgb {
        r.upload_texture(&png, &name)
    } else {
        r.upload_texture_with_format(&png, &name, wgpu::TextureFormat::Rgba8Unorm)
    }
    .map_err(|e| format!("texture {name}: {e}"))?;
    let table = if srgb {
        &mut cache.srgb
    } else {
        &mut cache.linear
    };
    table.insert(name, slot);
    Ok(slot)
}

fn material_of(
    r: &mut Renderer,
    cache: &mut TextureCache,
    document: &gltf::Document,
    images: &[gltf::image::Data],
    material: &gltf::Material<'_>,
    tint: Vec3,
) -> Result<usize, String> {
    let pbr = material.pbr_metallic_roughness();
    let masked = material.alpha_mode() != gltf::material::AlphaMode::Opaque;
    let mut m = Material::solid(tint);
    m.roughness = 1.0;
    m.metallic = 0.0;
    if let Some(info) = pbr.base_color_texture() {
        m.texture_index = Some(upload(
            r,
            cache,
            document,
            images,
            info.texture(),
            true,
            masked,
        )?);
    }
    if let Some(info) = material.normal_texture() {
        m.normal_texture_index = Some(upload(
            r,
            cache,
            document,
            images,
            info.texture(),
            false,
            false,
        )?);
    }
    if let Some(info) = pbr.metallic_roughness_texture() {
        m.mr_texture_index = Some(upload(
            r,
            cache,
            document,
            images,
            info.texture(),
            false,
            false,
        )?);
    }
    m.alpha_test = masked;
    Ok(r.add_material(m))
}

/// Per-vertex tangents from the UV layout (Lengyel), orthogonalised against the normal.
pub fn tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    idx: &[u32],
) -> Vec<[f32; 3]> {
    let mut acc = vec![Vec3::ZERO; positions.len()];
    for tri in idx.chunks_exact(3) {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let (p0, p1, p2) = (
            Vec3::from(positions[a]),
            Vec3::from(positions[b]),
            Vec3::from(positions[c]),
        );
        let (e1, e2) = (p1 - p0, p2 - p0);
        let (du1, dv1) = (uvs[b][0] - uvs[a][0], uvs[b][1] - uvs[a][1]);
        let (du2, dv2) = (uvs[c][0] - uvs[a][0], uvs[c][1] - uvs[a][1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1.0e-12 {
            continue;
        }
        let t = (e1 * dv2 - e2 * dv1) / det;
        acc[a] += t;
        acc[b] += t;
        acc[c] += t;
    }
    acc.iter()
        .zip(normals)
        .map(|(t, n)| {
            let n = Vec3::from(*n);
            let t = (*t - n * n.dot(*t)).normalize_or_zero();
            if t == Vec3::ZERO {
                let fallback = if n.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
                n.cross(fallback).normalize_or_zero().to_array()
            } else {
                t.to_array()
            }
        })
        .collect()
}

/// Cylindrical UVs around the Y axis for meshes exported without any (the trunks): the bark
/// repeats about every 2 m around and every 2 m along the trunk.
fn trunk_uvs(positions: &[[f32; 3]]) -> Vec<[f32; 2]> {
    positions
        .iter()
        .map(|p| [p[2].atan2(p[0]) / std::f32::consts::TAU * 2.0, p[1] * 0.5])
        .collect()
}

/// The `KHR_texture_transform` of the material (all its maps share one in Poly Haven exports),
/// baked into the UVs because the shader has no per-map transform.
fn uv_transform(material: &gltf::Material<'_>) -> Option<impl Fn([f32; 2]) -> [f32; 2]> {
    let transform = material
        .pbr_metallic_roughness()
        .base_color_texture()
        .and_then(|info| info.texture_transform())?;
    let (offset, rotation, scale) = (transform.offset(), transform.rotation(), transform.scale());
    let (sin, cos) = rotation.sin_cos();
    Some(move |uv: [f32; 2]| {
        let (x, y) = (uv[0] * scale[0], uv[1] * scale[1]);
        [
            cos * x + sin * y + offset[0],
            -sin * x + cos * y + offset[1],
        ]
    })
}

/// Loads `path` and uploads every primitive. Leaves and grass blades (alpha-masked materials)
/// get a second, back-facing copy because the forward pass culls back faces.
pub fn load(
    r: &mut Renderer,
    cache: &mut TextureCache,
    path: &Path,
    tint: Vec3,
    skip: Option<&str>,
) -> Result<Model, String> {
    let (document, buffers, images) =
        gltf::import(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut materials: HashMap<Option<usize>, usize> = HashMap::new();
    let mut parts = Vec::new();
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|b| Some(&buffers[b.index()]));
            let positions: Vec<[f32; 3]> = reader.read_positions().ok_or("no positions")?.collect();
            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()], Iterator::collect);
            let material = primitive.material();
            // Debug aid (`FOREST_SKIP`): drop primitives whose material name contains the text.
            if skip.is_some_and(|s| material.name().is_some_and(|n| n.contains(s))) {
                continue;
            }
            let has_uv = reader.read_tex_coords(0).is_some();
            let mut uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map_or_else(|| trunk_uvs(&positions), |t| t.into_f32().collect());
            if let Some(transform) = uv_transform(&material).filter(|_| has_uv) {
                for uv in &mut uvs {
                    *uv = transform(*uv);
                }
            }
            let mut indices: Vec<u32> = reader.read_indices().map_or_else(
                || (0..positions.len() as u32).collect(),
                |i| i.into_u32().collect(),
            );
            let tangent = tangents(&positions, &normals, &uvs, &indices);
            let mut vertices: Vec<Vertex> = (0..positions.len())
                .map(|i| Vertex {
                    position: positions[i],
                    normal: normals[i],
                    uv: uvs[i],
                    tangent: tangent[i],
                })
                .collect();

            let masked = material.alpha_mode() != gltf::material::AlphaMode::Opaque;
            if masked {
                let base = vertices.len() as u32;
                let back: Vec<Vertex> = vertices
                    .iter()
                    .map(|v| Vertex {
                        normal: [-v.normal[0], -v.normal[1], -v.normal[2]],
                        tangent: [-v.tangent[0], -v.tangent[1], -v.tangent[2]],
                        ..*v
                    })
                    .collect();
                let flipped: Vec<u32> = indices
                    .chunks_exact(3)
                    .flat_map(|t| [base + t[0], base + t[2], base + t[1]])
                    .collect();
                vertices.extend(back);
                indices.extend(flipped);
            }
            let slot = match materials.get(&material.index()) {
                Some(&slot) => slot,
                None => {
                    let slot = material_of(r, cache, &document, &images, &material, tint)?;
                    materials.insert(material.index(), slot);
                    slot
                }
            };
            let mesh_slot = r.upload_mesh(&Mesh::new(vertices, indices));
            parts.push(Part {
                mesh: mesh_slot,
                material: slot,
            });
        }
    }
    if parts.is_empty() {
        return Err(format!("{}: no meshes", path.display()));
    }
    Ok(Model { parts })
}
