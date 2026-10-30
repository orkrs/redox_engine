//! GPU copy of the global SDF volume (`sdf/global.rs`): an `R16Float` distance texture,
//! an `Rgba8Unorm` material texture, and a small uniform describing the volume.

use super::global::{GlobalSdf, MAX_DISTANCE};
use bytemuck::{Pod, Zeroable};

/// Volume descriptor mirrored by `struct SdfInfo` in `shaders/gi_trace.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GlobalSdfInfoGpu {
    /// Minimum corner of the volume (xyz) and the voxel size (w).
    pub origin_voxel: [f32; 4],
    /// Voxel counts (xyz); w is padding.
    pub dims: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<GlobalSdfInfoGpu>() == 32);

/// GPU-resident global SDF. Textures are sampled with a trilinear clamp sampler by the GI
/// trace; `distance` holds the signed distance, `albedo` the material (see `GlobalSdf`).
pub struct GlobalSdfGpu {
    /// Voxel counts per axis the textures were created with.
    pub dims: [u32; 3],
    /// `R16Float` 3D texture with the signed distance.
    pub distance_tex: wgpu::Texture,
    /// View of `distance_tex`.
    pub distance_view: wgpu::TextureView,
    /// `Rgba8Unorm` 3D texture with albedo (rgb) and emission scale (a).
    pub albedo_tex: wgpu::Texture,
    /// View of `albedo_tex`.
    pub albedo_view: wgpu::TextureView,
    /// Uniform buffer with [`GlobalSdfInfoGpu`].
    pub info: wgpu::Buffer,
}

impl GlobalSdfGpu {
    /// Creates the textures with every voxel at `MAX_DISTANCE` and no material, so the
    /// volume reads as empty space until the first upload.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, dims: [u32; 3]) -> Self {
        let size = wgpu::Extent3d {
            width: dims[0],
            height: dims[1],
            depth_or_array_layers: dims[2],
        };
        let distance_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("global_sdf_distance"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let albedo_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("global_sdf_albedo"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let info = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("global_sdf_info"),
            size: std::mem::size_of::<GlobalSdfInfoGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let this = Self {
            dims,
            distance_view: distance_tex.create_view(&Default::default()),
            albedo_view: albedo_tex.create_view(&Default::default()),
            distance_tex,
            albedo_tex,
            info,
        };
        let voxels = dims[0] as usize * dims[1] as usize * dims[2] as usize;
        let far = vec![half::f16::from_f32(MAX_DISTANCE).to_bits(); voxels];
        let empty = vec![[0u8; 4]; voxels];
        this.write_texels(queue, &far, &empty);
        queue.write_buffer(
            &this.info,
            0,
            bytemuck::bytes_of(&GlobalSdfInfoGpu {
                origin_voxel: [0.0, 0.0, 0.0, 1.0],
                dims: [dims[0], dims[1], dims[2], 0],
            }),
        );
        this
    }

    /// Replaces the volume contents with `sdf`. Recreates the textures when the resolution
    /// differs from the current one.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, sdf: &GlobalSdf) {
        if self.dims != sdf.dims {
            *self = Self::new(device, queue, sdf.dims);
        }
        self.write_texels(queue, &sdf.distance_f16_bits(), &sdf.albedo);
        queue.write_buffer(
            &self.info,
            0,
            bytemuck::bytes_of(&GlobalSdfInfoGpu {
                origin_voxel: [sdf.origin.x, sdf.origin.y, sdf.origin.z, sdf.voxel],
                dims: [sdf.dims[0], sdf.dims[1], sdf.dims[2], 0],
            }),
        );
    }

    fn write_texels(&self, queue: &wgpu::Queue, distance_bits: &[u16], albedo: &[[u8; 4]]) {
        let [dx, dy, dz] = self.dims;
        let size = wgpu::Extent3d {
            width: dx,
            height: dy,
            depth_or_array_layers: dz,
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.distance_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(distance_bits),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dx * 2),
                rows_per_image: Some(dy),
            },
            size,
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.albedo_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(albedo),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dx * 4),
                rows_per_image: Some(dy),
            },
            size,
        );
    }
}
