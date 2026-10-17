//! `bake-ibl`: turns an equirectangular HDR sky into four KTX2 files (feature `bake`).
//!
//! The GPU convolution that the renderer runs at startup is executed once here; a game then
//! loads the results with `IBLResource::from_ktx2` and skips the work.

use std::path::Path;

use redox_render::resource::ibl::{IBLProcessor, IBLResource, IblKtx2};
use redox_render::resource::texture::Texture;

/// File names written by [`bake_ibl`] (inside the output directory).
pub const OUTPUT_FILES: [&str; 4] = [
    "environment.ktx2",
    "irradiance.ktx2",
    "prefiltered.ktx2",
    "brdf_lut.ktx2",
];

/// Convolves `equirect_hdr` (a Radiance `.hdr` file) on the GPU and reads the maps back.
pub fn bake_ibl_maps(equirect_hdr: &[u8]) -> Result<IblKtx2, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| format!("no GPU adapter: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("redox_cook_bake"),
        ..Default::default()
    }))
    .map_err(|e| format!("cannot create a device: {e}"))?;

    let equirect = Texture::from_hdr_bytes(&device, &queue, equirect_hdr, "equirect")
        .map_err(|e| format!("cannot read the HDR image: {e}"))?;
    let processor = IBLProcessor::new(&device);
    let resource = IBLResource::from_equirect(&device, &queue, &processor, &equirect);
    resource
        .export_ktx2(&device, &queue)
        .ok_or_else(|| "the generated maps could not be read back".to_string())
}

/// Bakes `input` into `out_dir` (created if missing). Returns the written file names.
pub fn bake_ibl(input: &Path, out_dir: &Path) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let maps = bake_ibl_maps(&bytes)?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let textures = [
        &maps.environment,
        &maps.irradiance,
        &maps.prefiltered,
        &maps.brdf_lut,
    ];
    let mut written = Vec::new();
    for (name, texture) in OUTPUT_FILES.iter().zip(textures) {
        let path = out_dir.join(name);
        std::fs::write(&path, texture.to_ktx2_bytes(true))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        written.push((*name).to_string());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_asset::{AssetLoader, Ktx2Loader};

    #[test]
    fn baking_the_sample_sky_produces_loadable_files() {
        let sky = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/skybox.hdr");
        let out = std::env::temp_dir().join(format!("redox_bake_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        match bake_ibl(&sky, &out) {
            Err(e) if e.contains("no GPU") || e.contains("device") => {
                eprintln!("skipping: {e}");
                return;
            }
            other => assert_eq!(other.unwrap().len(), 4),
        }
        for name in OUTPUT_FILES {
            let bytes = std::fs::read(out.join(name)).unwrap();
            let tex = Ktx2Loader::default()
                .load(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(tex.is_cube(), name != "brdf_lut.ktx2", "{name}");
        }
        let _ = std::fs::remove_dir_all(&out);
    }
}
