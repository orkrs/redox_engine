//! Editor viewport mesh bridge: glTF payloads → GPU slots on the engine [`Renderer`].
//!
//! Uses [`redox_asset::load_scene_from_path`] (AST-01 loader). Materials use
//! factor colours plus embedded RGBA8 images from the same load when present.

use redox_asset::{GltfMeshData, GltfSceneData, load_scene_from_path};
use redox_math::{Aabb, Vec3};
use redox_render::{Material, Mesh, Renderer, mesh::Vertex, mesh::tangent};
use redox_scene::AssetRef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Kind of asset referenced from the scene (EDT-A8 merge: same fields as [`AssetRef`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ViewportAssetKind {
    /// Mesh asset (glTF / GLB / cooked mesh later).
    #[default]
    Mesh,
    /// Material asset.
    Material,
}

/// Minimal scene mesh reference when handles are not resolved yet.
///
/// **Merge assumption (EDT-A8):** replace with `redox_scene::AssetRef` only; paths
/// and GUIDs stay compatible with [`AssetRef::component`] == `"MeshHandle"`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewportSceneAssetRef {
    pub guid: Option<String>,
    pub path: Option<String>,
    pub kind: ViewportAssetKind,
}

impl ViewportSceneAssetRef {
    /// Builds from a reflected scene [`AssetRef`] when the component is a mesh handle.
    pub fn from_scene_ref(r: &AssetRef) -> Option<Self> {
        if r.component != "MeshHandle" {
            return None;
        }
        Some(Self {
            guid: r.guid.clone(),
            path: r.path.clone(),
            kind: ViewportAssetKind::Mesh,
        })
    }
}

/// One entity the viewport should draw.
#[derive(Clone, Debug)]
pub struct ViewportDrawItem {
    /// ECS entity id (for selection / missing overlay).
    pub entity_id: u32,
    pub transform: redox_math::Transform,
    /// Spawned glTF path from the asset browser (absolute or project-relative).
    pub mesh_path: Option<PathBuf>,
    /// Persisted GUID/path from scene [`AssetRefs`].
    pub mesh_ref: Option<ViewportSceneAssetRef>,
}

/// Resolved GPU resources for one cached file.
#[derive(Clone, Debug)]
pub struct ResolvedViewportMesh {
    pub mesh_slot: usize,
    pub material_slot: usize,
    pub local_bounds: Aabb,
    pub source_path: PathBuf,
    /// CPU mesh (for wireframe overlays).
    pub cpu_mesh: Mesh,
}

/// CPU-side cache key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    path: PathBuf,
    primitive: u32,
}

/// Load / GPU-upload cache for editor viewport meshes.
#[derive(Clone, Debug, Default)]
pub struct ViewportMeshCache {
    entries: HashMap<CacheKey, ResolvedViewportMesh>,
    missing: HashMap<u32, String>,
}

impl ViewportMeshCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Entity ids whose mesh could not be resolved (path errors, missing file).
    pub fn missing_entities(&self) -> &HashMap<u32, String> {
        &self.missing
    }

    /// Clears missing markers; call at the start of each resolve pass.
    pub fn clear_missing(&mut self) {
        self.missing.clear();
    }

    /// Returns GPU indices for `item`, uploading the mesh on first use.
    pub fn resolve(
        &mut self,
        renderer: &mut Renderer,
        assets_root: Option<&Path>,
        item: &ViewportDrawItem,
    ) -> Option<ResolvedViewportMesh> {
        let path = resolve_mesh_path(assets_root, item)?;
        let key = CacheKey {
            path: path.clone(),
            primitive: 0,
        };
        if let Some(hit) = self.entries.get(&key) {
            return Some(hit.clone());
        }
        match load_and_upload(renderer, &path) {
            Ok(resolved) => {
                let out = resolved.clone();
                self.entries.insert(key, resolved);
                Some(out)
            }
            Err(reason) => {
                self.missing.insert(item.entity_id, reason);
                None
            }
        }
    }
}

fn resolve_mesh_path(assets_root: Option<&Path>, item: &ViewportDrawItem) -> Option<PathBuf> {
    if let Some(p) = &item.mesh_path {
        if p.is_absolute() && p.is_file() {
            return Some(p.clone());
        }
        if let Some(root) = assets_root {
            let joined = root.join(p);
            if joined.is_file() {
                return Some(joined);
            }
        }
        if p.is_file() {
            return Some(p.clone());
        }
    }
    if let Some(ref r) = item.mesh_ref {
        if let Some(path) = &r.path {
            let pb = PathBuf::from(path);
            if pb.is_file() {
                return Some(pb);
            }
            if let Some(root) = assets_root {
                let joined = root.join(&pb);
                if joined.is_file() {
                    return Some(joined);
                }
            }
        }
    }
    None
}

fn load_and_upload(renderer: &mut Renderer, path: &Path) -> Result<ResolvedViewportMesh, String> {
    let scene = load_scene_from_path(path).map_err(|e| e.to_string())?;
    let mesh_idx = scene.nodes.iter().find_map(|n| n.mesh_index).unwrap_or(0);
    let mesh_data = scene
        .meshes
        .get(mesh_idx)
        .ok_or_else(|| "gltf scene has no meshes".to_string())?;
    let mesh = gltf_mesh_to_render_mesh(mesh_data);
    let mesh_slot = renderer.upload_mesh(&mesh);
    let mat_index = mesh_data.material.unwrap_or(0);
    let material_slot = material_from_gltf(renderer, &scene, mat_index);
    Ok(ResolvedViewportMesh {
        mesh_slot,
        material_slot,
        local_bounds: mesh.aabb,
        source_path: path.to_path_buf(),
        cpu_mesh: mesh,
    })
}

/// Converts [`GltfMeshData`] into engine [`Mesh`].
pub fn gltf_mesh_to_render_mesh(data: &GltfMeshData) -> Mesh {
    let n_verts = data.positions.len();
    let normals: Vec<[f32; 3]> = if data.normals.len() == n_verts {
        data.normals.clone()
    } else {
        vec![[0.0, 1.0, 0.0]; n_verts]
    };
    let uvs: Vec<[f32; 2]> = if data.uvs.len() == n_verts {
        data.uvs.clone()
    } else {
        vec![[0.0, 0.0]; n_verts]
    };
    let tangents: Vec<[f32; 3]> = if data.tangents.len() == n_verts {
        data.tangents.iter().map(|t| [t[0], t[1], t[2]]).collect()
    } else {
        tangent::compute_tangents(&data.positions, &normals, &uvs, &data.indices)
            .into_iter()
            .map(|t| [t[0], t[1], t[2]])
            .collect()
    };

    let vertices: Vec<Vertex> = (0..n_verts)
        .map(|i| Vertex {
            position: data.positions[i],
            normal: normals[i],
            uv: uvs[i],
            uv2: [0.0, 0.0],
            tangent: tangents[i],
        })
        .collect();
    Mesh::new(vertices, data.indices.clone())
}

fn material_from_gltf(renderer: &mut Renderer, scene: &GltfSceneData, index: usize) -> usize {
    let Some(src) = scene.materials.get(index) else {
        return renderer.add_material(Material::solid(Vec3::new(0.72, 0.74, 0.78)));
    };
    let mut material = Material::solid(Vec3::new(
        src.base_color_factor[0],
        src.base_color_factor[1],
        src.base_color_factor[2],
    ));
    material.roughness = src.roughness_factor;
    material.metallic = src.metallic_factor;
    material.emissive = Vec3::new(
        src.emissive_factor[0],
        src.emissive_factor[1],
        src.emissive_factor[2],
    );
    material.double_sided = src.double_sided;
    if let Some(tex) = &src.base_color {
        if let Some(img) = scene.textures.get(tex.image_index) {
            if let Ok(slot) = renderer.upload_texture(&img.rgba_bytes, "editor_gltf_albedo") {
                material.texture_index = Some(slot);
            }
        }
    }
    renderer.add_material(material)
}
