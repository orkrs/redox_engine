//! `bake-lightmap`: static mesh irradiance atlases.

use std::path::Path;

use redox_render::light::{DirectionalLight, LightScene};
use redox_render::lightmap::{
    LightmapBaker, LightmapEntry, LightmapScene, LightmapsManifest, reject_mesh_without_uv2,
    write_lightmaps_manifest,
};
use redox_render::mesh::{Mesh, Vertex};

/// One object to bake from a simple job file.
#[derive(Clone, Debug)]
pub struct LightmapJob {
    pub id: String,
    pub mesh: Mesh,
    pub skinned: bool,
}

/// Parses `lightmaps_jobs.ron` beside the world.
pub fn lightmap_jobs_from_world(world: &Path) -> Result<Vec<LightmapJob>, String> {
    let dir = world.parent().unwrap_or_else(|| Path::new("."));
    let path = dir.join("lightmaps_jobs.ron");
    if !path.is_file() {
        return Err(format!("no lightmaps_jobs.ron beside {}", world.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    #[derive(serde::Deserialize)]
    struct JobRon {
        objects: Vec<JobObj>,
    }
    #[derive(serde::Deserialize)]
    struct JobObj {
        id: String,
        skinned: bool,
        vertices: Vec<VertexRon>,
        indices: Vec<u32>,
    }
    #[derive(serde::Deserialize)]
    struct VertexRon {
        position: [f32; 3],
        normal: [f32; 3],
        uv: [f32; 2],
        uv2: [f32; 2],
        tangent: [f32; 3],
    }
    let parsed: JobRon = ron::from_str(&text).map_err(|e| e.to_string())?;
    Ok(parsed
        .objects
        .into_iter()
        .map(|o| LightmapJob {
            id: o.id,
            skinned: o.skinned,
            mesh: Mesh::new(
                o.vertices
                    .into_iter()
                    .map(|v| Vertex {
                        position: v.position,
                        normal: v.normal,
                        uv: v.uv,
                        uv2: v.uv2,
                        tangent: v.tangent,
                    })
                    .collect(),
                o.indices,
            ),
        })
        .collect())
}

/// Bakes lightmaps for objects with UV2 into `out_dir`.
pub fn bake_lightmap(world: &Path, out_dir: &Path) -> Result<Vec<String>, String> {
    let jobs = lightmap_jobs_from_world(world)?;
    let mut baked = 0usize;
    let mut skipped = Vec::new();
    let baker = LightmapBaker::default();
    let lights = LightScene::new(DirectionalLight::new(Vec3::new(0.2, -1.0, 0.0), Vec3::ONE, 2.0));
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut written = Vec::new();
    let meshes: Vec<Mesh> = jobs.iter().filter(|j| !j.skinned).map(|j| j.mesh.clone()).collect();
    let scene = LightmapScene {
        meshes: &meshes,
        lights: &lights,
    };

    for job in &jobs {
        if job.skinned {
            skipped.push(format!("{}: skinned mesh", job.id));
            continue;
        }
        if reject_mesh_without_uv2(&job.mesh).is_err() {
            skipped.push(format!("{}: missing UV2", job.id));
            continue;
        }
        let width = 32;
        let height = 32;
        let image = baker
            .bake_mesh(&job.mesh, &scene, width, height)
            .map_err(|e| format!("{}: {e:?}", job.id))?;
        let ktx = lightmap_image_to_ktx2(&image);
        let file = format!("{}.ktx2", job.id);
        std::fs::write(out_dir.join(&file), ktx.to_ktx2_bytes(true)).map_err(|e| e.to_string())?;
        entries.push(LightmapEntry {
            id: job.id.clone(),
            meters_per_texel: baker.meters_per_texel,
            path: file.clone(),
        });
        written.push(file);
        baked += 1;
    }
    for line in skipped {
        eprintln!("bake-lightmap skipped {line}");
    }
    if baked == 0 {
        return Err("no meshes with UV2 were baked".into());
    }
    write_lightmaps_manifest(out_dir, &LightmapsManifest { entries })?;
    written.push("lightmaps.ron".into());
    Ok(written)
}

fn lightmap_image_to_ktx2(image: &redox_render::lightmap::LightmapImage) -> redox_asset::Ktx2Texture {
    use redox_asset::TextureFormat as F;
    let mut packed = Vec::with_capacity(image.pixels.len() * 2);
    for &half in &image.pixels {
        packed.extend_from_slice(&half.to_le_bytes());
    }
    redox_asset::Ktx2Texture::new_2d(image.width, image.height, F::Rgba16Float, vec![packed])
}

/// Cook profile hook.
pub fn cook_profile_bake_lightmap(project_root: &Path) -> Option<String> {
    let world = project_root.join("assets/levels/main.scene.ron");
    if !world.is_file() {
        return Some("skipped".into());
    }
    let out = project_root.join(".redox/baked_lightmaps");
    match bake_lightmap(&world, &out) {
        Ok(_) => Some("written".into()),
        Err(e) if e.contains("lightmaps_jobs") => Some("skipped".into()),
        Err(e) => {
            eprintln!("bake-lightmap: {e}");
            Some("skipped".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bake_lightmap_writes_a_ktx2_for_a_static_mesh() {
        let dir = std::env::temp_dir().join(format!("bake_lm_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let jobs = r#"
(
    objects: [
        (
            id: "floor",
            skinned: false,
            vertices: [
                (position: (-1.0, 0.0, -1.0), normal: (0.0, 1.0, 0.0), uv: (0.0, 0.0), uv2: (0.1, 0.1), tangent: (1.0, 0.0, 0.0)),
                (position: (1.0, 0.0, -1.0), normal: (0.0, 1.0, 0.0), uv: (1.0, 0.0), uv2: (0.9, 0.1), tangent: (1.0, 0.0, 0.0)),
                (position: (1.0, 0.0, 1.0), normal: (0.0, 1.0, 0.0), uv: (1.0, 1.0), uv2: (0.9, 0.9), tangent: (1.0, 0.0, 0.0)),
                (position: (-1.0, 0.0, 1.0), normal: (0.0, 1.0, 0.0), uv: (0.0, 1.0), uv2: (0.1, 0.9), tangent: (1.0, 0.0, 0.0)),
            ],
            indices: [0, 1, 2, 0, 2, 3],
        ),
    ],
)
"#;
        std::fs::write(dir.join("lightmaps_jobs.ron"), jobs).unwrap();
        let world = dir.join("world.scene.ron");
        std::fs::write(&world, "").unwrap();
        let out = dir.join("out");
        let files = bake_lightmap(&world, &out).expect("bake");
        assert!(files.iter().any(|f| f == "floor.ktx2"));
        assert!(out.join("lightmaps.ron").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_skinned_mesh_is_skipped() {
        let dir = std::env::temp_dir().join(format!("bake_lm_skip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let jobs = r#"
(
    objects: [
        (id: "hero", skinned: true, vertices: [], indices: []),
        (
            id: "floor",
            skinned: false,
            vertices: [
                (position: (0.0, 0.0, 0.0), normal: (0.0, 1.0, 0.0), uv: (0.0, 0.0), uv2: (0.5, 0.5), tangent: (1.0, 0.0, 0.0)),
                (position: (1.0, 0.0, 0.0), normal: (0.0, 1.0, 0.0), uv: (1.0, 0.0), uv2: (0.5, 0.5), tangent: (1.0, 0.0, 0.0)),
                (position: (0.0, 0.0, 1.0), normal: (0.0, 1.0, 0.0), uv: (0.0, 1.0), uv2: (0.5, 0.5), tangent: (1.0, 0.0, 0.0)),
            ],
            indices: [0, 1, 2],
        ),
    ],
)
"#;
        std::fs::write(dir.join("lightmaps_jobs.ron"), jobs).unwrap();
        let world = dir.join("world.scene.ron");
        std::fs::write(&world, "").unwrap();
        bake_lightmap(&world, &dir.join("out")).expect("partial bake");
        let _ = std::fs::remove_dir_all(&dir);
    }
}