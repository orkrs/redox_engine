//! Opaque HDR scene colour with a mip chain for refractive lookups (MAT-09).

use crate::post::bloom::BLOOM_MIP_COUNT;

/// Mip levels in the scene-colour copy (same depth as the bloom chain).
pub const SCENE_COPY_MIP_COUNT: u32 = BLOOM_MIP_COUNT as u32;

/// Creates the scene-colour texture at internal resolution.
pub fn create_scene_copy_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene_copy"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: SCENE_COPY_MIP_COUNT,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// Per-mip views plus a view over the full chain (for `textureSampleLevel`).
pub fn scene_copy_views(
    texture: &wgpu::Texture,
) -> ([wgpu::TextureView; BLOOM_MIP_COUNT], wgpu::TextureView) {
    let mut mip_views = Vec::with_capacity(BLOOM_MIP_COUNT);
    for level in 0..BLOOM_MIP_COUNT {
        mip_views.push(texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("scene_copy_mip"),
            base_mip_level: level as u32,
            mip_level_count: Some(1),
            ..Default::default()
        }));
    }
    let chain = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("scene_copy_chain"),
        mip_level_count: Some(SCENE_COPY_MIP_COUNT),
        ..Default::default()
    });
    let arr: [wgpu::TextureView; BLOOM_MIP_COUNT] =
        mip_views.try_into().expect("scene copy mip view count");
    (arr, chain)
}

/// Records bloom-style downsamples from mip 0 into coarser mips (no threshold).
pub fn record_scene_copy_mips(
    encoder: &mut wgpu::CommandEncoder,
    bloom_pass: &crate::post::bloom::BloomPass,
    down_bgs: &[wgpu::BindGroup],
    mip_views: &[wgpu::TextureView; BLOOM_MIP_COUNT],
) {
    for i in 1..BLOOM_MIP_COUNT {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene_copy_mipgen"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: &mip_views[i],
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&bloom_pass.downsample_pipeline);
        pass.set_bind_group(0, &down_bgs[i], &[]);
        pass.draw(0..3, 0..1);
    }
}
