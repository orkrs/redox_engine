//! Packs albedo / normal / MR / lightmap into one `texture_2d_array` (LGT-01 + sampled-texture budget).

use crate::resource::texture::Texture;

/// Layer indices in the material array (WGSL must match).
pub const MATERIAL_LAYER_ALBEDO: u32 = 0;
pub const MATERIAL_LAYER_NORMAL: u32 = 1;
pub const MATERIAL_LAYER_MR: u32 = 2;
pub const MATERIAL_LAYER_LIGHTMAP: u32 = 3;
pub const MATERIAL_LAYER_COUNT: u32 = 4;

fn copy_layer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Texture,
    dst: &wgpu::Texture,
    layer: u32,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("material_layer_copy"),
    });
    let copy_w = src.width().min(dst.width());
    let copy_h = src.height().min(dst.height());
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: src,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: copy_w,
            height: copy_h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
}

fn write_layer_1x1(queue: &wgpu::Queue, dst: &wgpu::Texture, layer: u32, rgba: [u8; 4]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

/// Builds a four-layer material array. Uses the albedo texture format; smaller layers are
/// copied or filled with their 1×1 fallback texels.
pub fn build_material_layer_array(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    albedo: &Texture,
    normal: &Texture,
    mr: &Texture,
    lightmap: &Texture,
    label: &str,
) -> wgpu::Texture {
    let width = albedo
        .width
        .max(normal.width)
        .max(mr.width)
        .max(lightmap.width)
        .max(1);
    let height = albedo
        .height
        .max(normal.height)
        .max(mr.height)
        .max(lightmap.height)
        .max(1);
    let format = albedo.texture.format();

    let array = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: MATERIAL_LAYER_COUNT,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let layers: [(&Texture, u32, [u8; 4]); 4] = [
        (albedo, MATERIAL_LAYER_ALBEDO, [255, 255, 255, 255]),
        (normal, MATERIAL_LAYER_NORMAL, [128, 128, 255, 255]),
        (mr, MATERIAL_LAYER_MR, [255, 255, 0, 255]),
        (lightmap, MATERIAL_LAYER_LIGHTMAP, [0, 0, 0, 0]),
    ];

    for (src, layer, fallback_rgba) in layers {
        if src.texture.format() == format {
            copy_layer(device, queue, &src.texture, &array, layer);
        } else if src.width == 1 && src.height == 1 {
            write_layer_1x1(queue, &array, layer, fallback_rgba);
        }
    }

    array
}

/// View of all four material layers.
pub fn material_layer_array_view(texture: &wgpu::Texture, label: &str) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(label),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        array_layer_count: Some(MATERIAL_LAYER_COUNT),
        ..Default::default()
    })
}
