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
        "tone_mapping",
        "vg_resolve",
        "taa",
        "velocity",
        "debug_lines",
        "not_a_shader",
    ]);
    assert_eq!(rebuilt.len(), 13);
    renderer
        .render_frame_into(&[], &view)
        .expect("frame after reload");
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "validation error: {error:?}");
}

/// Restores a shader file when dropped, even if the test panics.
#[cfg(feature = "shader-hot-reload")]
struct Restore(std::path::PathBuf, String);

#[cfg(feature = "shader-hot-reload")]
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, &self.1);
    }
}

/// Edit a real shader file while the renderer runs: the watcher notices it, the pass is rebuilt
/// from the edited file, and a broken edit is reported without touching the pass.
#[cfg(feature = "shader-hot-reload")]
#[test]
fn editing_a_shader_file_rebuilds_its_pass() {
    use redox_render::renderer::{ReloadablePass, ShaderHotReload};

    let Some(mut renderer) = renderer() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/velocity.wgsl");
    let original = std::fs::read_to_string(&path).unwrap();
    let _restore = Restore(path.clone(), original.clone());

    let mut hot = ShaderHotReload::new();
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert!(hot.update(&mut renderer).is_empty(), "nothing changed yet");

    // A harmless edit rebuilds the pass.
    std::fs::write(&path, format!("{original}\n// edited at runtime\n")).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_eq!(hot.update(&mut renderer), vec![ReloadablePass::Velocity]);

    // A broken edit is reported by validation and the pass is left alone.
    std::fs::write(&path, format!("{original}\nthis is not wgsl\n")).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert!(
        hot.update(&mut renderer).is_empty(),
        "broken shader must not be applied"
    );
}
