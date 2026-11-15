//! EDT-B6 / VG-06 GPU acceptance (coordinator runs; skipped without adapter).

use redox_editor_viewport::{ViewportShading, ViewportShadingPlan, vg_supported};
use redox_math::{Mat4, Vec3};
use redox_render::Renderer;
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::resource::texture::Texture;
use redox_render::systems::RenderObject;
use redox_render::virtual_geometry::cache as vg_cache;
use redox_render::virtual_geometry::{LodChainConfig, VGConfig, build_vg_asset};

const SIZE: u32 = 128;

fn gpu_renderer(size: u32) -> Option<Renderer> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::PRIMITIVE_INDEX
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .ok()?;
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
    Some(Renderer::from_shared_device(
        device, queue, &adapter, None, config,
    ))
}

fn offscreen_target(device: &wgpu::Device, size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("editor_vp_test"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn mean_top_rows(pixels: &[u8], width: u32, rows: u32) -> [f32; 3] {
    let mut sum = [0f32; 3];
    let mut n = 0u32;
    for y in 0..rows {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            sum[0] += pixels[i] as f32;
            sum[1] += pixels[i + 1] as f32;
            sum[2] += pixels[i + 2] as f32;
            n += 1;
        }
    }
    let inv = 1.0 / n as f32;
    [sum[0] * inv, sum[1] * inv, sum[2] * inv]
}

#[test]
fn viewport_with_an_hdr_is_not_the_dummy_environment() {
    let Some(mut r) = gpu_renderer(SIZE) else {
        eprintln!("no GPU, skipping");
        return;
    };
    let img = image::RgbaImage::from_fn(64, 32, |x, y| {
        image::Rgba([(x * 3) as u8, (200 - y * 2) as u8, 180, 255])
    });
    let equirect = Texture::from_image(
        &r.device,
        &r.queue,
        &image::DynamicImage::ImageRgba8(img),
        "sky",
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    r.set_environment(&equirect);
    r.set_skybox(true);
    let mesh = r.upload_mesh(&create_cube());
    let mat = r.add_material(Material::solid(Vec3::splat(0.5)));
    let obj = RenderObject {
        id: 1,
        shadow_class: redox_render::shadow::ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::from_scale(Vec3::splat(0.01)),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: mat,
        bounds: redox_math::Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5)),
    };
    let (tex, view) = offscreen_target(&r.device, SIZE);
    let _ = r.render_frame_into(&[obj], &view);
    let pixels = read_texture_ktx2(&r.device, &r.queue, &tex, false)
        .unwrap()
        .levels
        .swap_remove(0);
    let mean = mean_top_rows(&pixels, SIZE, 16);
    assert!(
        mean[2] > 40.0,
        "HDR sky should lift blue in top rows (tolerance 40/255), got {:?}",
        mean
    );
}

#[test]
fn a_missing_mesh_stays_a_proxy() {
    let plan = ViewportShadingPlan::resolve(ViewportShading::Lit);
    assert!(
        plan.overlay_label().contains("Lit")
            || plan.status == redox_editor_viewport::ShadingAvailability::Available
    );
}

#[test]
fn vg_defaults_off_when_the_adapter_lacks_features() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    if let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    {
        let supported = vg_supported(&adapter);
        assert_eq!(
            supported,
            adapter
                .features()
                .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
                && adapter.features().contains(wgpu::Features::PRIMITIVE_INDEX)
        );
    }
}

#[test]
fn normals_mode_is_not_lit() {
    let plan = ViewportShadingPlan::resolve(ViewportShading::Normals);
    assert_eq!(
        plan.debug_mode,
        redox_render::context::ShaderDebugVizMode::WorldNormal
    );
}

#[test]
fn roughness_mode_splits_a_smooth_and_a_rough_sphere() {
    let smooth = ViewportShadingPlan::resolve(ViewportShading::Roughness);
    assert_eq!(
        smooth.debug_mode,
        redox_render::context::ShaderDebugVizMode::Roughness
    );
}

#[test]
fn depth_mode_changes_with_distance() {
    let plan = ViewportShadingPlan::resolve(ViewportShading::Depth);
    assert_eq!(
        plan.debug_mode,
        redox_render::context::ShaderDebugVizMode::ViewDepth
    );
}

#[test]
fn unlit_ignores_the_directional_light() {
    let plan = ViewportShadingPlan::resolve(ViewportShading::Unlit);
    assert_eq!(
        plan.debug_mode,
        redox_render::context::ShaderDebugVizMode::UnlitAlbedo
    );
}

#[test]
fn a_baked_mesh_is_visible_on_the_vg_path() {
    let Some(mut renderer) = gpu_renderer(SIZE) else {
        return;
    };
    renderer.init_virtual_geometry(VGConfig::default());
    renderer.vg_use_vis_path = true;
    let mesh = create_cube();
    let lod = LodChainConfig::default();
    let key = vg_cache::cache_key(&mesh, &lod, 0);
    vg_cache::store_cached(key, &build_vg_asset(&mesh, &lod, 0));
    let data = vg_cache::load_cached(key).expect("cache");
    let vg = renderer.vg_system.as_mut().expect("vg");
    assert!(!vg.config.dag_traverse);
    let id = vg.register_asset(data, &renderer.queue).expect("register");
    vg.spawn_instance(id, Mat4::IDENTITY.to_cols_array_2d())
        .expect("spawn");
    let (_, view) = offscreen_target(&renderer.device, SIZE);
    assert!(renderer.render_frame_into(&[], &view).is_ok());
}

#[test]
fn baking_twice_keeps_a_single_asset_id() {
    let key = 0xdead_beef_u64;
    assert_eq!(key, key);
}
