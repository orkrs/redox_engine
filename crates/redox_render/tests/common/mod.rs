//! Shared GPU test setup (tests are skipped when no adapter exists).
#![allow(dead_code)]

pub fn adapter() -> Option<wgpu::Adapter> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
}

pub fn device_with_bc(adapter: &wgpu::Adapter) -> Option<(wgpu::Device, wgpu::Queue)> {
    let wanted = wgpu::Features::PRIMITIVE_INDEX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::TEXTURE_COMPRESSION_BC
        | wgpu::Features::PIPELINE_CACHE
        | redox_render::profiler::PROFILER_FEATURES;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        ..Default::default()
    }))
    .ok()
}

pub fn renderer(adapter: &wgpu::Adapter, size: u32) -> Option<redox_render::Renderer> {
    let (device, queue) = device_with_bc(adapter)?;
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
    Some(redox_render::Renderer::from_shared_device(
        device, queue, adapter, None, config,
    ))
}

pub fn target(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
