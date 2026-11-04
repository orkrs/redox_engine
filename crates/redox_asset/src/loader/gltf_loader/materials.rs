//! glTF material conversion: PBR factors, texture slots with their UV set and
//! `KHR_texture_transform`, alpha mode and sidedness.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::GltfMaterialData;

/// Reference from a material slot to an image of the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GltfTextureRef {
    /// Index into [`super::GltfSceneData::textures`] (document image order).
    pub image_index: usize,
    /// UV set the slot samples (`texCoord`, overridden by the transform).
    pub tex_coord: u32,
    /// `KHR_texture_transform` of the slot, if present.
    pub transform: Option<GltfUvTransform>,
}

/// `KHR_texture_transform`, as authored (not yet applied to the UVs).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GltfUvTransform {
    /// Translation applied after scale and rotation.
    pub offset: [f32; 2],
    /// Rotation in radians (counter-clockwise).
    pub rotation: f32,
    /// Scale applied before rotation.
    pub scale: [f32; 2],
}

/// `alphaMode` of a material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GltfAlphaMode {
    /// Fully opaque (the glTF default).
    #[default]
    Opaque,
    /// Alpha-tested against `alpha_cutoff`.
    Mask,
    /// Alpha-blended.
    Blend,
}

/// Extracts PBR parameters, texture slots and render state from a material.
/// The base-color path is set only for external URI textures.
pub(super) fn convert_material(
    material: &gltf::Material<'_>,
    base_dir: Option<&Path>,
) -> GltfMaterialData {
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
        base_color: pbr.base_color_texture().map(|info| texture_ref(&info)),
        normal: material.normal_texture().map(|normal| GltfTextureRef {
            image_index: normal.texture().source().index(),
            tex_coord: normal.tex_coord(),
            transform: None,
        }),
        metallic_roughness: pbr
            .metallic_roughness_texture()
            .map(|info| texture_ref(&info)),
        occlusion: material
            .occlusion_texture()
            .map(|occlusion| GltfTextureRef {
                image_index: occlusion.texture().source().index(),
                tex_coord: occlusion.tex_coord(),
                transform: None,
            }),
        emissive: material.emissive_texture().map(|info| texture_ref(&info)),
        alpha_mode: alpha_mode(material.alpha_mode()),
        alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
        double_sided: material.double_sided(),
        emissive_factor: material.emissive_factor(),
    }
}

/// Reads one `textureInfo` slot. A `KHR_texture_transform` with its own
/// `texCoord` overrides the slot's UV set, as the extension specifies.
fn texture_ref(info: &gltf::texture::Info<'_>) -> GltfTextureRef {
    let mut tex_coord = info.tex_coord();
    let transform = info.texture_transform().map(|t| {
        if let Some(set) = t.tex_coord() {
            tex_coord = set;
        }
        GltfUvTransform {
            offset: t.offset(),
            rotation: t.rotation(),
            scale: t.scale(),
        }
    });
    GltfTextureRef {
        image_index: info.texture().source().index(),
        tex_coord,
        transform,
    }
}

fn alpha_mode(mode: gltf::material::AlphaMode) -> GltfAlphaMode {
    match mode {
        gltf::material::AlphaMode::Opaque => GltfAlphaMode::Opaque,
        gltf::material::AlphaMode::Mask => GltfAlphaMode::Mask,
        gltf::material::AlphaMode::Blend => GltfAlphaMode::Blend,
    }
}
