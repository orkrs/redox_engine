//! Free helper functions for render-target creation and culling.

use super::*;

/// Helper to create a render target texture for normal or GTAO pass.
pub(super) fn create_normal_ssao_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Pick the best HDR MSAA sample count supported by the adapter (prefer ×4, then ×2).
///
/// Used by [`crate::Renderer`] and editor offscreen hosts that share the Sharp AA path.
pub fn pick_hdr_msaa_sample_count(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    want_msaa: bool,
) -> u32 {
    if !want_msaa {
        return 1;
    }
    let flags = adapter.get_texture_format_features(format).flags;
    for count in [4u32, 2] {
        if flags.sample_count_supported(count) {
            return count;
        }
    }
    1
}

/// Rgba8Unorm intermediate for Sharp AA (LDR tonemap output or SMAA result).
pub(super) fn create_sharp_intermediate(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Previous-frame depth buffer for TAA disocclusion (Depth32Float copy).
pub(super) fn create_prev_depth_texture(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("prev_depth_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: crate::pass::forward::DEPTH_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Helper to create HDR texture, view and sampler.
pub(super) fn create_hdr_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("hdr_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    (texture, view, sampler)
}

/// Multisampled HDR colour target for geometry passes (TXAA-style MSAA input).
/// Includes TEXTURE_BINDING so the Smart Resolve pass can read individual samples.
pub(super) fn create_hdr_msaa_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    sample_count: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr_msaa_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Multisampled depth target matching HDR MSAA sample count.
pub(super) fn create_depth_msaa_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    sample_count: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth_msaa_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: crate::pass::forward::DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

pub(super) fn create_bloom_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

pub(super) fn create_bloom_mip_chain(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (
    [wgpu::Texture; crate::post::bloom::BLOOM_MIP_COUNT],
    [wgpu::TextureView; crate::post::bloom::BLOOM_MIP_COUNT],
) {
    use crate::post::bloom::BLOOM_MIP_COUNT;
    let mut textures = Vec::with_capacity(BLOOM_MIP_COUNT);
    let mut views = Vec::with_capacity(BLOOM_MIP_COUNT);
    let mut w = width;
    let mut h = height;
    for i in 0..BLOOM_MIP_COUNT {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        let (tex, view) = create_bloom_texture(device, w, h, format, &format!("bloom_mip{i}"));
        textures.push(tex);
        views.push(view);
    }
    (
        textures.try_into().unwrap(),
        views.try_into().unwrap(),
    )
}

pub(super) fn create_bloom_downsample_bgs(
    bloom_pass: &crate::post::bloom::BloomPass,
    device: &wgpu::Device,
    hdr_view: &wgpu::TextureView,
    mip_views: &[wgpu::TextureView; crate::post::bloom::BLOOM_MIP_COUNT],
    sampler: &wgpu::Sampler,
) -> [wgpu::BindGroup; crate::post::bloom::BLOOM_MIP_COUNT] {
    use crate::post::bloom::BLOOM_MIP_COUNT;
    let mut bgs = Vec::with_capacity(BLOOM_MIP_COUNT);
    for i in 0..BLOOM_MIP_COUNT {
        let src = if i == 0 { hdr_view } else { &mip_views[i - 1] };
        bgs.push(bloom_pass.create_bind_group(device, src, sampler));
    }
    bgs.try_into().unwrap()
}

pub(super) fn create_bloom_upsample_bgs(
    bloom_pass: &crate::post::bloom::BloomPass,
    device: &wgpu::Device,
    mip_views: &[wgpu::TextureView; crate::post::bloom::BLOOM_MIP_COUNT],
    sampler: &wgpu::Sampler,
) -> [wgpu::BindGroup; crate::post::bloom::BLOOM_MIP_COUNT - 1] {
    use crate::post::bloom::BLOOM_MIP_COUNT;
    let mut bgs = Vec::with_capacity(BLOOM_MIP_COUNT - 1);
    for i in 0..BLOOM_MIP_COUNT - 1 {
        // Upsample reads the coarser mip (i+1) into mip i.
        bgs.push(bloom_pass.create_bind_group(device, &mip_views[i + 1], sampler));
    }
    bgs.try_into().unwrap()
}

/// Conservative overlap test: world AABB vs ortho clip cube `[-1,1]×[-1,1]×[0,1]`.
pub(super) fn aabb_overlaps_clip(bounds: Aabb, view_proj: Mat4) -> bool {
    let b = bounds;
    let corners = [
        Vec3::new(b.min.x, b.min.y, b.min.z),
        Vec3::new(b.max.x, b.min.y, b.min.z),
        Vec3::new(b.min.x, b.max.y, b.min.z),
        Vec3::new(b.max.x, b.max.y, b.min.z),
        Vec3::new(b.min.x, b.min.y, b.max.z),
        Vec3::new(b.max.x, b.min.y, b.max.z),
        Vec3::new(b.min.x, b.max.y, b.max.z),
        Vec3::new(b.max.x, b.max.y, b.max.z),
    ];
    let mut min_c = Vec3::splat(f32::INFINITY);
    let mut max_c = Vec3::splat(f32::NEG_INFINITY);
    for c in corners {
        let p = view_proj.transform_point3(c);
        min_c = min_c.min(p);
        max_c = max_c.max(p);
    }
    !(max_c.x < -1.0
        || min_c.x > 1.0
        || max_c.y < -1.0
        || min_c.y > 1.0
        || max_c.z < 0.0
        || min_c.z > 1.0)
}
