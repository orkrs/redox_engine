//! `bake-probes`: offline reflection probe cubes (feature `bake`).

use std::path::{Path, PathBuf};

use redox_math::{Quat, Vec3};
use redox_render::Renderer;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::reflection::{
    ProbeBakeEntry, ProbesManifest, ReflectionProbe, export_slot_prefiltered, write_probe,
    write_probes_manifest,
};
use redox_render::resource::ibl::{IBLProcessor, IBLResource};
use redox_render::resource::texture::Texture;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const FACE_SIZE: u32 = 128;

/// Reads probe boxes from `probes.ron` beside `world` (same directory).
pub fn probes_from_world(world: &Path) -> Result<ProbesManifest, String> {
    let dir = world
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let manifest_path = dir.join("probes.ron");
    if !manifest_path.is_file() {
        return Err(format!(
            "no probes.ron next to {} (place probe metadata beside the world file)",
            world.display()
        ));
    }
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    ron::from_str(&text).map_err(|e| e.to_string())
}

/// Bakes probes listed in the manifest next to `world` into `out_dir`.
#[cfg(feature = "bake")]
pub fn bake_probes(world: &Path, out_dir: &Path) -> Result<Vec<String>, String> {
    let manifest = probes_from_world(world)?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| format!("no GPU adapter: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("redox_cook_bake_probes"),
        ..Default::default()
    }))
    .map_err(|e| format!("cannot create a device: {e}"))?;

    let size = (FACE_SIZE * 2).max(256);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width: size,
        height: size,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    let mut renderer = Renderer::from_shared_device(device, queue, &adapter, None, config);

    let sky = image::RgbaImage::from_pixel(64, 32, image::Rgba([40, 50, 80, 255]));
    let equirect = Texture::from_image(
        &renderer.device,
        &renderer.queue,
        &image::DynamicImage::ImageRgba8(sky),
        "bake_sky",
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .map_err(|e| format!("sky: {e}"))?;
    let ibl = IBLResource::from_equirect(
        &renderer.device,
        &renderer.queue,
        &IBLProcessor::new(&renderer.device),
        &equirect,
    );
    renderer.set_ibl_resource(ibl);
    renderer.set_reflection_probes_enabled(true);

    let cube = renderer.upload_mesh(&create_cube());
    let floor_mat = renderer.add_material(Material::solid(Vec3::splat(0.5)).roughness(0.4));
    let objects = vec![RenderObject {
        id: 1,
        shadow_class: ShadowClass::Static,
        deforms: false,
        pose_version: 0,
        model_matrix: redox_math::Mat4::from_scale_rotation_translation(
            Vec3::new(20.0, 0.1, 20.0),
            Quat::IDENTITY,
            Vec3::new(0.0, -1.0, 0.0),
        ),
        color: [1.0; 4],
        mesh_index: cube,
        material_index: floor_mat,
        bounds: redox_math::Aabb::from_center_size(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(20.0, 0.1, 20.0),
        ),
    }];
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.2, -1.0, 0.1),
        Vec3::ONE,
        2.0,
    ));

    let probes: Vec<ReflectionProbe> = manifest
        .probes
        .iter()
        .map(ProbeBakeEntry::to_probe)
        .collect();
    renderer.set_reflection_probes(&probes);

    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut written = Vec::new();
    for (slot, entry) in manifest.probes.iter().enumerate() {
        renderer
            .capture_reflection_probe(slot, &objects, &lights)
            .map_err(|e| e.to_string())?;
        let ktx = export_slot_prefiltered(
            &renderer.device,
            &renderer.queue,
            renderer.reflection_probes(),
            slot,
        )
        .ok_or_else(|| format!("probe {} readback failed", entry.id))?;
        write_probe(out_dir, entry, &ktx).map_err(|e| e.to_string())?;
        written.push(format!("{}.ktx2", entry.id));
    }
    write_probes_manifest(out_dir, &manifest).map_err(|e| e.to_string())?;
    written.push("probes.ron".into());
    Ok(written)
}

#[cfg(not(feature = "bake"))]
pub fn bake_probes(_world: &Path, _out_dir: &Path) -> Result<Vec<String>, String> {
    Err(
        "bake-probes needs the GPU renderer: cargo run -p redox_cook --features bake -- bake-probes ..."
            .into(),
    )
}

/// Cook profile hook: returns `"written"` / `"skipped"` / `None`.
pub fn cook_profile_bake_probes(project_root: &Path) -> Option<String> {
    let world = find_world_probes(project_root)?;
    let out = project_root.join(".redox/baked_probes");
    match bake_probes(&world, &out) {
        Ok(_) => Some("written".into()),
        Err(e) if e.contains("no probes.ron") => Some("skipped".into()),
        Err(e) if e.contains("no GPU") || e.contains("adapter") => Some("skipped".into()),
        Err(e) => {
            eprintln!("bake-probes: {e}");
            Some("skipped".into())
        }
    }
}

fn find_world_probes(root: &Path) -> Option<PathBuf> {
    let scene = root.join("assets/levels/main.scene.ron");
    if scene.is_file() {
        return Some(scene);
    }
    let manifest = root.join("probes.ron");
    manifest.is_file().then(|| root.join("world.scene.ron"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_render::reflection::{ProbeBakeEntry, write_probes_manifest, ProbesManifest};

    #[test]
    fn bake_probes_writes_one_file_per_probe() {
        let dir = std::env::temp_dir().join(format!("bake_probes_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = ProbesManifest {
            probes: vec![ProbeBakeEntry {
                id: "p0".into(),
                center: [0.0, 1.0, 0.0],
                half_extent: [5.0, 5.0, 5.0],
                blend_distance: 1.0,
                priority: 0,
            }],
        };
        write_probes_manifest(&dir, &manifest).unwrap();
        let world = dir.join("world.scene.ron");
        std::fs::write(&world, "").unwrap();
        match bake_probes(&world, &dir.join("out")) {
            Err(e) if e.contains("no GPU") || e.contains("adapter") => {
                eprintln!("skipping GPU bake-probes: {e}");
            }
            Ok(files) => {
                assert!(files.iter().any(|f| f == "p0.ktx2"));
                assert!(files.iter().any(|f| f == "probes.ron"));
            }
            Err(e) => panic!("{e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}