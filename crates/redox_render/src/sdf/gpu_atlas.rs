//! GPU 128³ SDF brick atlas with a multi-brick page table for PBR sampling.
//!
//! The atlas packs up to [`MAX_SDF_BRICKS`] 64³ bricks into a 2×2×2 page grid
//! inside the 128³ volume. [`SdfSceneGpu`] carries the page table so
//! `pbr.wgsl` / `sdf_shadow.wgsl` can sample more than one brick.

use bytemuck::{Pod, Zeroable};

use super::SdfBrick;
use super::bake::SDF_RESOLUTION;

/// Side length of the global 3D atlas texture (texels per axis).
pub const GPU_ATLAS_EXTENT: u32 = 128;

/// Maximum bricks packed into the 128³ atlas (2×2×2 pages of 64³).
pub const MAX_SDF_BRICKS: usize = 8;

/// One page-table entry: world AABB + atlas texel origin.
/// Mirrored in `pbr.wgsl` / `sdf_shadow.wgsl` as `SdfBrickPageGpu`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfBrickPageGpu {
    /// World-space AABB min (xyz); w unused.
    pub world_min: [f32; 4],
    /// World-space AABB max (xyz); w unused.
    pub world_max: [f32; 4],
    /// Atlas texel origin (xyz) + brick resolution in texels (w, typically 64).
    pub atlas_origin_res: [f32; 4],
}

impl Default for SdfBrickPageGpu {
    fn default() -> Self {
        Self {
            world_min: [0.0; 4],
            world_max: [0.0; 4],
            atlas_origin_res: [0.0, 0.0, 0.0, SDF_RESOLUTION as f32],
        }
    }
}

/// Scene descriptor mirrored in `pbr.wgsl` / `sdf_shadow.wgsl` (`@binding(26)`).
///
/// Layout (16-byte aligned):
/// * `atlas_dims` — x = brick count, yzw reserved / brick resolution in w
/// * `world_bounds_min` / `world_bounds_max_texel` — union AABB + texel size in w
/// * `pages` — up to [`MAX_SDF_BRICKS`] page-table entries
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfSceneGpu {
    pub atlas_dims: [u32; 4],
    pub world_bounds_min: [f32; 4],
    pub world_bounds_max_texel: [f32; 4],
    pub pages: [SdfBrickPageGpu; MAX_SDF_BRICKS],
}

/// Single-channel 128³ distance-field atlas on the GPU.
pub struct SdfGpuAtlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub scene_buffer: wgpu::Buffer,
    pub scene: SdfSceneGpu,
    pub enabled: bool,
}

impl SdfGpuAtlas {
    /// Creates an empty atlas (large positive distances) with a default scene uniform.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let extent = GPU_ATLAS_EXTENT;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sdf_brick_atlas_128"),
            size: wgpu::Extent3d {
                width: extent,
                height: extent,
                depth_or_array_layers: extent,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("sdf_brick_atlas_view"),
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sdf_brick_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let far = 1.0e6_f32;
        let voxels = (extent * extent * extent) as usize;
        let init_data = vec![far; voxels];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&init_data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(extent * 4),
                rows_per_image: Some(extent),
            },
            wgpu::Extent3d {
                width: extent,
                height: extent,
                depth_or_array_layers: extent,
            },
        );

        let scene = SdfSceneGpu {
            atlas_dims: [0, 0, 0, SDF_RESOLUTION as u32],
            world_bounds_min: [0.0; 4],
            world_bounds_max_texel: [1.0, 1.0, 1.0, 0.01],
            pages: [SdfBrickPageGpu::default(); MAX_SDF_BRICKS],
        };
        let scene_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sdf_scene_uniform"),
            size: std::mem::size_of::<SdfSceneGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&scene_buffer, 0, bytemuck::bytes_of(&scene));

        Self {
            texture,
            view,
            sampler,
            scene_buffer,
            scene,
            enabled: false,
        }
    }

    /// Packs one brick into page slot 0 (compat helper).
    pub fn upload_brick(&mut self, queue: &wgpu::Queue, brick: &SdfBrick) {
        self.upload_bricks(queue, std::slice::from_ref(brick));
    }

    /// Packs up to [`MAX_SDF_BRICKS`] bricks into the 2×2×2 page grid and uploads
    /// the page table + atlas voxels.
    pub fn upload_bricks(&mut self, queue: &wgpu::Queue, bricks: &[SdfBrick]) {
        let count = bricks.len().min(MAX_SDF_BRICKS);
        if count == 0 {
            self.scene.atlas_dims[0] = 0;
            queue.write_buffer(&self.scene_buffer, 0, bytemuck::bytes_of(&self.scene));
            self.enabled = false;
            return;
        }

        let res = SDF_RESOLUTION as u32;
        let pages_per_axis = 2u32; // 128 / 64
        let mut pages = [SdfBrickPageGpu::default(); MAX_SDF_BRICKS];
        let mut bounds_min = bricks[0].origin_ws - bricks[0].half_size();
        let mut bounds_max = bricks[0].origin_ws + bricks[0].half_size();
        let mut min_texel = bricks[0].field.texel_world_size().x;

        // Clear atlas to "far outside".
        let mut voxels =
            vec![1.0e6_f32; (GPU_ATLAS_EXTENT * GPU_ATLAS_EXTENT * GPU_ATLAS_EXTENT) as usize];

        for (i, brick) in bricks.iter().take(count).enumerate() {
            let px = (i as u32) % pages_per_axis;
            let py = ((i as u32) / pages_per_axis) % pages_per_axis;
            let pz = (i as u32) / (pages_per_axis * pages_per_axis);
            let origin_x = px * res;
            let origin_y = py * res;
            let origin_z = pz * res;

            let half = brick.half_size();
            let bmin = brick.origin_ws - half;
            let bmax = brick.origin_ws + half;
            bounds_min = bounds_min.min(bmin);
            bounds_max = bounds_max.max(bmax);
            min_texel = min_texel.min(brick.field.texel_world_size().x);

            pages[i] = SdfBrickPageGpu {
                world_min: [bmin.x, bmin.y, bmin.z, 0.0],
                world_max: [bmax.x, bmax.y, bmax.z, 0.0],
                atlas_origin_res: [
                    origin_x as f32,
                    origin_y as f32,
                    origin_z as f32,
                    res as f32,
                ],
            };

            for z in 0..SDF_RESOLUTION {
                for y in 0..SDF_RESOLUTION {
                    for x in 0..SDF_RESOLUTION {
                        let src = super::bake::DistanceField64::index(x, y, z);
                        let dst_x = origin_x as usize + x;
                        let dst_y = origin_y as usize + y;
                        let dst_z = origin_z as usize + z;
                        let dst = (dst_z * GPU_ATLAS_EXTENT as usize + dst_y)
                            * GPU_ATLAS_EXTENT as usize
                            + dst_x;
                        voxels[dst] = brick.field.data[src];
                    }
                }
            }
        }

        self.scene = SdfSceneGpu {
            atlas_dims: [
                count as u32,
                pages_per_axis,
                pages_per_axis,
                SDF_RESOLUTION as u32,
            ],
            world_bounds_min: [bounds_min.x, bounds_min.y, bounds_min.z, 0.0],
            world_bounds_max_texel: [bounds_max.x, bounds_max.y, bounds_max.z, min_texel],
            pages,
        };
        queue.write_buffer(&self.scene_buffer, 0, bytemuck::bytes_of(&self.scene));

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&voxels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(GPU_ATLAS_EXTENT * 4),
                rows_per_image: Some(GPU_ATLAS_EXTENT),
            },
            wgpu::Extent3d {
                width: GPU_ATLAS_EXTENT,
                height: GPU_ATLAS_EXTENT,
                depth_or_array_layers: GPU_ATLAS_EXTENT,
            },
        );
        self.enabled = true;
    }
}

const _: () = {
    assert!(std::mem::size_of::<SdfBrickPageGpu>() == 48);
    assert!(std::mem::size_of::<SdfSceneGpu>() == 48 + 48 * MAX_SDF_BRICKS);
};
