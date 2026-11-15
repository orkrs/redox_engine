//! On-disk reflection probe assets (`<id>.ktx2` + `probes.ron`).

use std::path::Path;

use redox_math::Vec3;
use serde::{Deserialize, Serialize};

use super::probe::ReflectionProbe;
use super::set::{PROBE_FACE_SIZE, PROBE_FORMAT, PROBE_MIP_LEVELS, ReflectionProbeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProbeBakeEntry {
    pub id: String,
    pub center: [f32; 3],
    pub half_extent: [f32; 3],
    pub blend_distance: f32,
    pub priority: i32,
}

impl ProbeBakeEntry {
    pub fn to_probe(&self) -> ReflectionProbe {
        ReflectionProbe {
            center: Vec3::from_array(self.center),
            half_extent: Vec3::from_array(self.half_extent),
            blend_distance: self.blend_distance,
            priority: self.priority,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProbesManifest {
    pub probes: Vec<ProbeBakeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFileError {
    InvalidKtx2(String),
    Manifest(String),
}

impl std::fmt::Display for ProbeFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKtx2(msg) => write!(f, "invalid probe ktx2: {msg}"),
            Self::Manifest(msg) => write!(f, "invalid probes manifest: {msg}"),
        }
    }
}

impl std::error::Error for ProbeFileError {}

pub fn read_probe_prefiltered(bytes: &[u8]) -> Result<redox_asset::Ktx2Texture, ProbeFileError> {
    if bytes.len() < 64 {
        return Err(ProbeFileError::InvalidKtx2("truncated header".into()));
    }
    let reader =
        ktx2::Reader::new(bytes).map_err(|e| ProbeFileError::InvalidKtx2(format!("{e:?}")))?;
    let header = reader.header();
    if header.face_count != 6
        || header.pixel_width != PROBE_FACE_SIZE
        || header.pixel_height != PROBE_FACE_SIZE
        || header.level_count != PROBE_MIP_LEVELS
    {
        return Err(ProbeFileError::InvalidKtx2("unexpected layout".into()));
    }
    use redox_asset::AssetLoader;
    redox_asset::Ktx2Loader::default()
        .load(bytes)
        .map_err(|e| ProbeFileError::InvalidKtx2(e.to_string()))
}

pub fn write_probe(
    out_dir: &Path,
    entry: &ProbeBakeEntry,
    prefiltered: &redox_asset::Ktx2Texture,
) -> Result<std::path::PathBuf, ProbeFileError> {
    std::fs::create_dir_all(out_dir).map_err(|e| ProbeFileError::Manifest(e.to_string()))?;
    let path = out_dir.join(format!("{}.ktx2", entry.id));
    std::fs::write(&path, prefiltered.to_ktx2_bytes(true))
        .map_err(|e| ProbeFileError::Manifest(e.to_string()))?;
    Ok(path)
}

pub fn write_probes_manifest(
    out_dir: &Path,
    manifest: &ProbesManifest,
) -> Result<(), ProbeFileError> {
    std::fs::create_dir_all(out_dir).map_err(|e| ProbeFileError::Manifest(e.to_string()))?;
    let text = ron::ser::to_string_pretty(manifest, ron::ser::PrettyConfig::new())
        .map_err(|e| ProbeFileError::Manifest(e.to_string()))?;
    std::fs::write(out_dir.join("probes.ron"), text)
        .map_err(|e| ProbeFileError::Manifest(e.to_string()))
}

pub fn read_probes_manifest(dir: &Path) -> Result<ProbesManifest, ProbeFileError> {
    let text = std::fs::read_to_string(dir.join("probes.ron"))
        .map_err(|e| ProbeFileError::Manifest(e.to_string()))?;
    ron::from_str(&text).map_err(|e| ProbeFileError::Manifest(e.to_string()))
}

impl ReflectionProbeSet {
    pub fn load_baked(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dir: &Path,
    ) -> Result<usize, ProbeFileError> {
        let manifest = read_probes_manifest(dir)?;
        let probes: Vec<ReflectionProbe> = manifest
            .probes
            .iter()
            .map(ProbeBakeEntry::to_probe)
            .collect();
        let count = self.set_probes(&probes);
        for (slot, entry) in manifest.probes.iter().take(count).enumerate() {
            let bytes = std::fs::read(dir.join(format!("{}.ktx2", entry.id)))
                .map_err(|e| ProbeFileError::Manifest(e.to_string()))?;
            let ktx = read_probe_prefiltered(&bytes)?;
            upload_baked_slot(device, queue, self.texture(), slot, &ktx)?;
            self.baked_slots[slot] = true;
        }
        Ok(count)
    }

    pub fn slot_is_baked(&self, slot: usize) -> bool {
        self.baked_slots.get(slot).copied().unwrap_or(false)
    }
}

fn upload_baked_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    array: &wgpu::Texture,
    slot: usize,
    ktx: &redox_asset::Ktx2Texture,
) -> Result<(), ProbeFileError> {
    let Some(tex) = crate::resource::texture::Texture::from_ktx2(device, queue, ktx, "baked_probe")
    else {
        return Err(ProbeFileError::InvalidKtx2("unsupported format".into()));
    };
    if tex.texture.format() != PROBE_FORMAT {
        return Err(ProbeFileError::InvalidKtx2("wrong format".into()));
    }
    copy_cube_into_slot(device, queue, array, slot, &tex.texture);
    Ok(())
}

pub fn export_slot_prefiltered(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    set: &ReflectionProbeSet,
    slot: usize,
) -> Option<redox_asset::Ktx2Texture> {
    let cube = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("probe_export_cube"),
        size: wgpu::Extent3d {
            width: PROBE_FACE_SIZE,
            height: PROBE_FACE_SIZE,
            depth_or_array_layers: 6,
        },
        mip_level_count: PROBE_MIP_LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PROBE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    copy_cube_from_slot(device, queue, set.texture(), slot, &cube);
    crate::resource::ibl::read_texture_ktx2(device, queue, &cube, true)
}

fn copy_cube_into_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    array: &wgpu::Texture,
    slot: usize,
    cube: &wgpu::Texture,
) {
    copy_faces(device, queue, cube, 0, array, slot as u32 * 6);
}

fn copy_cube_from_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    array: &wgpu::Texture,
    slot: usize,
    cube: &wgpu::Texture,
) {
    copy_faces(device, queue, array, slot as u32 * 6, cube, 0);
}

fn copy_faces(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Texture,
    src_base: u32,
    dst: &wgpu::Texture,
    dst_base: u32,
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    for mip in 0..PROBE_MIP_LEVELS {
        let size = (PROBE_FACE_SIZE >> mip).max(1);
        for face in 0..6 {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: src,
                    mip_level: mip,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: src_base + face,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: dst,
                    mip_level: mip,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: dst_base + face,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    queue.submit(Some(encoder.finish()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_file_roundtrips_the_box_metadata() {
        let manifest = ProbesManifest {
            probes: vec![ProbeBakeEntry {
                id: "lobby".into(),
                center: [1.0, 2.0, 3.0],
                half_extent: [4.0, 5.0, 6.0],
                blend_distance: 0.5,
                priority: 2,
            }],
        };
        let dir = std::env::temp_dir().join(format!("probe_ron_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_probes_manifest(&dir, &manifest).unwrap();
        assert_eq!(read_probes_manifest(&dir).unwrap(), manifest);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_ktx2_is_an_error() {
        assert!(matches!(
            read_probe_prefiltered(&[0u8; 8]).unwrap_err(),
            ProbeFileError::InvalidKtx2(_)
        ));
    }
}
