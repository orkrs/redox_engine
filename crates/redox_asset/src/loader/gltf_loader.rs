//! glTF 2.0 scene loader.
//!
//! Loads `.gltf` / `.glb` files into plain, serde-serializable
//! [`GltfSceneData`] payloads (meshes, materials, nodes, textures).
//!
//! The asset crate cannot depend on `redox_render`, so this loader
//! intentionally does **not** produce render-side `MeshData`/`MaterialData`.
//! A render-side bridge (owned by `redox_render`, wired separately) converts
//! these payloads into GPU uploads. Current payload conventions the bridge
//! must know about:
//!
//! - One [`GltfMeshData`] per primitive; when a mesh has multiple
//!   primitives they are suffixed `.p{index}` for uniqueness.
//! - Node transforms are stored exactly as authored (local TRS); composing
//!   world matrices from the hierarchy is the consumer's job.
//! - `GltfMaterialData::base_color_texture_path` is only populated for
//!   external URI textures; embedded textures are exposed positionally
//!   through [`GltfSceneData::textures`] (document image order). Linking
//!   embedded textures to materials is deferred to the render-side bridge.
//! - External file references (separate `.bin` buffers, external image
//!   files) are resolved by [`load_scene_from_path`] / [`AssetLoader::load_from_path`]
//!   relative to the `.gltf` directory. Byte-only [`load_scene_from_bytes`] still
//!   requires embedded `data:` URIs or a `.glb`.
//! - Tangents: an authored `TANGENT` attribute wins; otherwise tangents are
//!   generated with Lengyel's method. The algorithm is duplicated locally
//!   from `redox_render::mesh::tangent::compute_tangents` (the dependency
//!   direction forbids sharing); both copies are unit-tested.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AssetError;
use crate::loader::AssetLoader;

/// One renderable surface: a flattened primitive of a glTF mesh.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GltfMeshData {
    /// Mesh name (or synthesized `mesh_{index}`), suffixed `.p{n}` when the
    /// source mesh has multiple primitives.
    pub name: String,
    /// Vertex positions, tightly packed.
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals; defaults to straight up when the file omits them.
    pub normals: Vec<[f32; 3]>,
    /// Primary UV set; zeros when the file omits it.
    pub uvs: Vec<[f32; 2]>,
    /// Per-vertex tangents `[tx, ty, tz, handedness]`.
    pub tangents: Vec<[f32; 4]>,
    /// Triangle indices (three per triangle); sequential when the primitive
    /// is non-indexed.
    pub indices: Vec<u32>,
}

/// PBR material parameters of a glTF primitive.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GltfMaterialData {
    /// `baseColorFactor` (RGBA, linear).
    pub base_color_factor: [f32; 4],
    /// Path of the base-color texture when it is an external file reference;
    /// `None` for embedded textures or when the slot is unused.
    pub base_color_texture_path: Option<PathBuf>,
    /// `metallicFactor`.
    pub metallic_factor: f32,
    /// `roughnessFactor`.
    pub roughness_factor: f32,
}

/// A scene-graph node flattened in document order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GltfNodeData {
    /// Node name (or synthesized `node_{index}`).
    pub name: String,
    /// Local translation `[x, y, z]`.
    pub translation: [f32; 3],
    /// Local rotation quaternion `[x, y, z, w]`.
    pub rotation_quat: [f32; 4],
    /// Local scale `[x, y, z]`.
    pub scale: [f32; 3],
    /// Index into [`GltfSceneData::meshes`] when the node carries a mesh.
    pub mesh_index: Option<usize>,
}

/// A decoded texture image converted to RGBA8.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GltfTextureData {
    /// Image name (or synthesized `texture_{index}`).
    pub name: String,
    /// Decoded pixels, RGBA8, row-major, tightly packed.
    pub rgba_bytes: Vec<u8>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Full payload produced by [`GltfLoader`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GltfSceneData {
    /// All mesh primitives in document order.
    pub meshes: Vec<GltfMeshData>,
    /// All materials in document order.
    pub materials: Vec<GltfMaterialData>,
    /// All nodes flattened in document order.
    pub nodes: Vec<GltfNodeData>,
    /// All images in document order, converted to RGBA8.
    pub textures: Vec<GltfTextureData>,
}

/// Loads glTF 2.0 scenes (`.gltf` JSON or binary `.glb`) from raw bytes.
///
/// Prefer [`AssetLoader::load_from_path`] for `.gltf` files with sibling
/// `.bin` / `.png` URIs. Byte-only loading supports `.glb` and `data:` URIs.
pub struct GltfLoader;

impl AssetLoader for GltfLoader {
    type Asset = GltfSceneData;

    fn load(&self, bytes: &[u8]) -> Result<Self::Asset, AssetError> {
        load_scene_from_bytes(bytes)
    }

    fn load_from_path(&self, path: &Path, bytes: &[u8]) -> Result<Self::Asset, AssetError> {
        match load_scene_from_path(path) {
            Ok(scene) => Ok(scene),
            Err(_) => load_scene_from_bytes(bytes),
        }
    }

    fn extensions(&self) -> Vec<&'static str> {
        vec!["gltf", "glb"]
    }
}

/// Parses a glTF document from a filesystem path, resolving sibling buffers
/// and external image URIs relative to the file's directory.
pub fn load_scene_from_path(path: &Path) -> Result<GltfSceneData, AssetError> {
    let (document, buffers, images) = gltf::import(path)
        .map_err(|e| AssetError::Gltf(format!("glTF import failed ({path:?}): {e}")))?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    assemble_scene(document, buffers, images, Some(base_dir))
}

/// Parses and converts a glTF document plus its embedded resources.
pub fn load_scene_from_bytes(bytes: &[u8]) -> Result<GltfSceneData, AssetError> {
    let (document, buffers, images) = gltf::import_slice(bytes)
        .map_err(|e| AssetError::Gltf(format!("glTF import failed: {e}")))?;
    assemble_scene(document, buffers, images, None)
}

fn assemble_scene(
    document: gltf::Document,
    buffers: Vec<gltf::buffer::Data>,
    images: Vec<gltf::image::Data>,
    base_dir: Option<&Path>,
) -> Result<GltfSceneData, AssetError> {
    let mut scene = GltfSceneData::default();

    for mesh in document.meshes() {
        let base_name = mesh
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("mesh_{}", mesh.index()));
        let primitive_count = mesh.primitives().count();
        for primitive in mesh.primitives() {
            scene.meshes.push(read_primitive(
                &primitive,
                &buffers,
                &base_name,
                primitive_count > 1,
            )?);
        }
    }

    for material in document.materials() {
        scene.materials.push(convert_material(&material, base_dir));
    }

    for node in document.nodes() {
        let (translation, rotation_quat, scale) = match node.transform() {
            gltf::scene::Transform::Decomposed {
                translation,
                rotation,
                scale,
            } => (translation, rotation, scale),
            gltf::scene::Transform::Matrix { matrix } => decompose_trs(matrix),
        };
        scene.nodes.push(GltfNodeData {
            name: node
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("node_{}", node.index())),
            translation,
            rotation_quat,
            scale,
            mesh_index: node.mesh().map(|mesh| mesh.index()),
        });
    }

    for (index, image_data) in images.iter().enumerate() {
        let name = document
            .images()
            .nth(index)
            .and_then(|image| image.name())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("texture_{index}"));
        scene.textures.push(GltfTextureData {
            name,
            width: image_data.width,
            height: image_data.height,
            rgba_bytes: pixels_to_rgba8(
                &image_data.pixels,
                image_data.format,
                image_data.width,
                image_data.height,
            )?,
        });
    }

    Ok(scene)
}

/// Flattens one primitive into a [`GltfMeshData`], filling sensible defaults
/// for optional attributes and computing tangents when absent.
fn read_primitive(
    primitive: &gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
    base_name: &str,
    suffix: bool,
) -> Result<GltfMeshData, AssetError> {
    let name = if suffix {
        format!("{base_name}.p{}", primitive.index())
    } else {
        base_name.to_owned()
    };

    let reader =
        primitive.reader(|buffer| buffers.get(buffer.index()).map(|data| data.0.as_slice()));

    // POSITION is the one attribute the spec requires.
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| {
            AssetError::LoadFailed(format!("glTF primitive '{name}' has no POSITION attribute"))
        })?
        .collect();
    let vertex_count = positions.len();

    let mut normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(Iterator::collect)
        .unwrap_or_default();
    if normals.is_empty() && vertex_count > 0 {
        // Match the OBJ loader convention: default up normal when absent.
        normals = vec![[0.0, 1.0, 0.0]; vertex_count];
    }

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|tex_coords| tex_coords.into_f32().collect())
        .unwrap_or_default();
    let uvs = if uvs.len() == vertex_count {
        uvs
    } else {
        vec![[0.0, 0.0]; vertex_count]
    };

    let indices: Vec<u32> = match reader.read_indices() {
        Some(read) => read.into_u32().collect(),
        None => (0..vertex_count as u32).collect(), // non-indexed primitive
    };

    // Authored tangents win; otherwise generate with Lengyel's method.
    let mut tangents: Vec<[f32; 4]> = reader
        .read_tangents()
        .map(Iterator::collect)
        .unwrap_or_default();
    if tangents.len() != vertex_count {
        tangents = compute_tangents(&positions, &normals, &uvs, &indices);
    }

    Ok(GltfMeshData {
        name,
        positions,
        normals,
        uvs,
        tangents,
        indices,
    })
}

/// Extracts PBR parameters and the base-color texture reference (external
/// URIs only) from a glTF material.
fn convert_material(material: &gltf::Material<'_>, base_dir: Option<&Path>) -> GltfMaterialData {
    let pbr = material.pbr_metallic_roughness();
    let base_color_texture_path =
        pbr.base_color_texture()
            .and_then(|info| match info.texture().source().source() {
                gltf::image::Source::Uri { uri, .. } => {
                    let p = PathBuf::from(uri);
                    Some(match base_dir {
                        Some(dir) if !p.is_absolute() => dir.join(p),
                        _ => p,
                    })
                }
                gltf::image::Source::View { .. } => None,
            });
    GltfMaterialData {
        base_color_factor: pbr.base_color_factor(),
        base_color_texture_path,
        metallic_factor: pbr.metallic_factor(),
        roughness_factor: pbr.roughness_factor(),
    }
}

/// Converts decoded glTF image pixels to tightly packed RGBA8.
fn pixels_to_rgba8(
    pixels: &[u8],
    format: gltf::image::Format,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, AssetError> {
    use gltf::image::Format;

    let (channels, bytes_per_channel) = match format {
        Format::R8 => (1usize, 1usize),
        Format::R8G8 => (2, 1),
        Format::R8G8B8 => (3, 1),
        Format::R8G8B8A8 => (4, 1),
        Format::R16 => (1, 2),
        Format::R16G16 => (2, 2),
        Format::R16G16B16 => (3, 2),
        Format::R16G16B16A16 => (4, 2),
        Format::R32G32B32FLOAT => (3, 4),
        Format::R32G32B32A32FLOAT => (4, 4),
    };
    let pixel_count = width as usize * height as usize;
    if pixels.len() < pixel_count * channels * bytes_per_channel {
        return Err(AssetError::LoadFailed(format!(
            "glTF texture buffer too small: {} bytes for {width}x{height} {format:?}",
            pixels.len()
        )));
    }

    let mut rgba = Vec::with_capacity(pixel_count * 4);
    match format {
        Format::R8 => {
            for &v in pixels.iter().take(pixel_count) {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Format::R8G8 => {
            for p in 0..pixel_count {
                // Luminance-alpha: replicate luminance, keep alpha.
                let (l, a) = (pixels[p * 2], pixels[p * 2 + 1]);
                rgba.extend_from_slice(&[l, l, l, a]);
            }
        }
        Format::R8G8B8 => {
            for p in 0..pixel_count {
                let (r, g, b) = (pixels[p * 3], pixels[p * 3 + 1], pixels[p * 3 + 2]);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Format::R8G8B8A8 => rgba.extend_from_slice(&pixels[..pixel_count * 4]),
        Format::R16 => {
            for p in 0..pixel_count {
                let v = narrow16(u16_at(pixels, p));
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        Format::R16G16 => {
            for p in 0..pixel_count {
                let l = narrow16(u16_at(pixels, p * 2));
                let a = narrow16(u16_at(pixels, p * 2 + 1));
                rgba.extend_from_slice(&[l, l, l, a]);
            }
        }
        Format::R16G16B16 => {
            for p in 0..pixel_count {
                let r = narrow16(u16_at(pixels, p * 3));
                let g = narrow16(u16_at(pixels, p * 3 + 1));
                let b = narrow16(u16_at(pixels, p * 3 + 2));
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Format::R16G16B16A16 => {
            for p in 0..pixel_count {
                let r = narrow16(u16_at(pixels, p * 4));
                let g = narrow16(u16_at(pixels, p * 4 + 1));
                let b = narrow16(u16_at(pixels, p * 4 + 2));
                let a = narrow16(u16_at(pixels, p * 4 + 3));
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
        Format::R32G32B32FLOAT => {
            for p in 0..pixel_count {
                let r = narrow_f32(f32_at(pixels, p * 3));
                let g = narrow_f32(f32_at(pixels, p * 3 + 1));
                let b = narrow_f32(f32_at(pixels, p * 3 + 2));
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Format::R32G32B32A32FLOAT => {
            for p in 0..pixel_count {
                let r = narrow_f32(f32_at(pixels, p * 4));
                let g = narrow_f32(f32_at(pixels, p * 4 + 1));
                let b = narrow_f32(f32_at(pixels, p * 4 + 2));
                let a = narrow_f32(f32_at(pixels, p * 4 + 3));
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
    }
    Ok(rgba)
}

/// Reads the little-endian u16 at `index`.
#[inline]
fn u16_at(pixels: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([pixels[index * 2], pixels[index * 2 + 1]])
}

/// Reads the little-endian f32 at `index`.
#[inline]
fn f32_at(pixels: &[u8], index: usize) -> f32 {
    f32::from_le_bytes(
        pixels[index * 4..index * 4 + 4]
            .try_into()
            .expect("length checked"),
    )
}

/// Scales 16-bit unorm to 8-bit unorm.
#[inline]
fn narrow16(value: u16) -> u8 {
    ((value as u32 * 255 + 32_767) / 65_535) as u8
}

/// Decomposes a column-major 4x4 transform matrix into translation,
/// rotation quaternion `[x, y, z, w]`, and scale. Used for nodes authored
/// with a raw `"matrix"` instead of TRS. Mirrored matrices (negative
/// determinant) cannot be represented exactly as TRS and yield an
/// approximate rotation.
fn decompose_trs(m: [[f32; 4]; 4]) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let translation = [m[3][0], m[3][1], m[3][2]];

    // Basis images are the columns of the upper-left 3x3 block.
    let mut scale = [
        len3([m[0][0], m[0][1], m[0][2]]),
        len3([m[1][0], m[1][1], m[1][2]]),
        len3([m[2][0], m[2][1], m[2][2]]),
    ];
    // Guard against singular matrices so the division below stays finite.
    for s in &mut scale {
        if *s < f32::EPSILON {
            *s = 1.0;
        }
    }

    // Normalized rotation matrix, stored row-major for quaternion extraction.
    let r = [
        [m[0][0] / scale[0], m[1][0] / scale[1], m[2][0] / scale[2]],
        [m[0][1] / scale[0], m[1][1] / scale[1], m[2][1] / scale[2]],
        [m[0][2] / scale[0], m[1][2] / scale[1], m[2][2] / scale[2]],
    ];

    (translation, quat_from_rot_matrix(&r), scale)
}

/// Extracts a unit quaternion `[x, y, z, w]` from a row-major rotation
/// matrix (Shepperd's method).
fn quat_from_rot_matrix(r: &[[f32; 3]; 3]) -> [f32; 4] {
    let trace = r[0][0] + r[1][1] + r[2][2];
    let (x, y, z, w) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        (
            (r[2][1] - r[1][2]) / s,
            (r[0][2] - r[2][0]) / s,
            (r[1][0] - r[0][1]) / s,
            0.25 * s,
        )
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        (
            0.25 * s,
            (r[0][1] + r[1][0]) / s,
            (r[0][2] + r[2][0]) / s,
            (r[2][1] - r[1][2]) / s,
        )
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        (
            (r[0][1] + r[1][0]) / s,
            0.25 * s,
            (r[1][2] + r[2][1]) / s,
            (r[0][2] - r[2][0]) / s,
        )
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        (
            (r[0][2] + r[2][0]) / s,
            (r[1][2] + r[2][1]) / s,
            0.25 * s,
            (r[1][0] - r[0][1]) / s,
        )
    };

    let norm = (x * x + y * y + z * z + w * w).sqrt();
    if norm > f32::EPSILON {
        [x / norm, y / norm, z / norm, w / norm]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

#[inline]
fn len3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Clamps a float to [0, 1] and quantizes to 8-bit unorm.
#[inline]
fn narrow_f32(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Lengyel tangent generation, local copy of
/// `redox_render::mesh::tangent::compute_tangents`. Kept here because
/// `redox_asset` cannot depend on `redox_render`; keep the two in sync
/// (both are unit-tested against the same fixtures).
fn compute_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    const FALLBACK: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

    let count = positions.len();
    let mut tan_acc = vec![[0.0f32; 3]; count];
    let mut bitan_acc = vec![[0.0f32; 3]; count];

    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= count || i1 >= count || i2 >= count {
            continue;
        }

        let p0 = positions[i0];
        let p1 = positions[i1];
        let p2 = positions[i2];
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

        let (u0, v0) = uvs.get(i0).map(|uv| (uv[0], uv[1])).unwrap_or((0.0, 0.0));
        let (u1, v1) = uvs.get(i1).map(|uv| (uv[0], uv[1])).unwrap_or((0.0, 0.0));
        let (u2, v2) = uvs.get(i2).map(|uv| (uv[0], uv[1])).unwrap_or((0.0, 0.0));
        let du1 = u1 - u0;
        let dv1 = v1 - v0;
        let du2 = u2 - u0;
        let dv2 = v2 - v0;

        let det = du1 * dv2 - du2 * dv1;
        if det.abs() <= f32::EPSILON {
            continue;
        }
        let r = 1.0 / det;

        let t = [
            r * (dv2 * e1[0] - dv1 * e2[0]),
            r * (dv2 * e1[1] - dv1 * e2[1]),
            r * (dv2 * e1[2] - dv1 * e2[2]),
        ];
        let b = [
            r * (-du2 * e1[0] + du1 * e2[0]),
            r * (-du2 * e1[1] + du1 * e2[1]),
            r * (-du2 * e1[2] + du1 * e2[2]),
        ];

        for &i in &[i0, i1, i2] {
            tan_acc[i][0] += t[0];
            tan_acc[i][1] += t[1];
            tan_acc[i][2] += t[2];
            bitan_acc[i][0] += b[0];
            bitan_acc[i][1] += b[1];
            bitan_acc[i][2] += b[2];
        }
    }

    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let n = normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
        let n_len_sq = dot3(n, n);
        let n = if n_len_sq > f32::EPSILON {
            [
                n[0] / n_len_sq.sqrt(),
                n[1] / n_len_sq.sqrt(),
                n[2] / n_len_sq.sqrt(),
            ]
        } else {
            [0.0, 1.0, 0.0]
        };

        let d = dot3(n, tan_acc[i]);
        let t = [
            tan_acc[i][0] - n[0] * d,
            tan_acc[i][1] - n[1] * d,
            tan_acc[i][2] - n[2] * d,
        ];
        let t_len_sq = dot3(t, t);
        if t_len_sq <= f32::EPSILON {
            result.push(FALLBACK);
            continue;
        }
        let inv_len = 1.0 / t_len_sq.sqrt();
        let t = [t[0] * inv_len, t[1] * inv_len, t[2] * inv_len];

        let c = [
            n[1] * t[2] - n[2] * t[1],
            n[2] * t[0] - n[0] * t[2],
            n[0] * t[1] - n[1] * t[0],
        ];
        let w = if dot3(c, bitan_acc[i]) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        result.push([t[0], t[1], t[2], w]);
    }
    result
}

#[inline]
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests;
