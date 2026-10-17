//! Rebuilding passes with `Renderer::reload_shaders` keeps the frame valid (skipped without a GPU).

use redox_render::Renderer;

fn renderer() -> Option<Renderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    let wanted = wgpu::Features::PRIMITIVE_INDEX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::PIPELINE_CACHE
        | redox_render::profiler::PROFILER_FEATURES;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        ..Default::default()
    }))
    .ok()?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width: 320,
        height: 200,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    Some(Renderer::from_shared_device(
        device, queue, &adapter, None, config,
    ))
}

#[test]
fn reloaded_passes_render_a_valid_frame() {
    let Some(mut renderer) = renderer() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: 320,
            height: 200,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());

    let scope = renderer
        .device
        .push_error_scope(wgpu::ErrorFilter::Validation);
    renderer
        .render_frame_into(&[], &view)
        .expect("frame before reload");
    let rebuilt = renderer.reload_shaders(&[
        "pbr",
        "shadow",
        "normal",
        "gtao",
        "vsm_shadow_trace",
        "scatter_light",
        "bloom",
        "msaa_resolve",
        "taa",
        "velocity",
        "debug_lines",
        "not_a_shader",
    ]);
    assert_eq!(rebuilt.len(), 11);
    renderer
        .render_frame_into(&[], &view)
        .expect("frame after reload");
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "validation error: {error:?}");
}
