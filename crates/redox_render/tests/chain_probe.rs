//! One red cube through the same path Courier uses: TAA on, sharp flag still set,
//! SSR + motion blur + froxel fog, pool rebuilt after those settings (the resize).
//!
//! Prints a number for every target the frame writes. Skipped without a GPU.
//!
//! ```text
//! cargo test -p redox_render --test chain_probe -- --test-threads=1 --nocapture
//! ```

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::post::fog::FogSettings;
use redox_render::post::post_fx::MotionBlurSettings;
use redox_render::post::ssr::SsrSettings;
use redox_render::systems::RenderObject;

const SIZE: u32 = 64;
const FRAMES: u32 = 8;

#[test]
fn red_cube_reports_every_target() {
    let _gpu = common::lock_gpu();
    let Some(adapter) = common::adapter() else {
        eprintln!("00 no adapter, skipped");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("00 no device, skipped");
        return;
    };

    eprintln!(
        "01 renderer  {SIZE}×{SIZE}  taa={} sharp={} msaa={}",
        renderer.taa_enabled, renderer.aa_sharp, renderer.msaa_sample_count
    );

    renderer.set_ssr(Some(SsrSettings {
        max_roughness: 0.45,
        intensity: 0.7,
        ..Default::default()
    }));
    renderer.set_fog(Some(FogSettings {
        color: [0.62, 0.72, 0.85],
        density: 0.012,
        height_falloff: 0.2,
        sun_scatter: 0.6,
        ..Default::default()
    }));
    renderer.set_fog_froxel(true);
    renderer.set_motion_blur(Some(MotionBlurSettings {
        shutter: 0.35,
        max_pixels: 14.0,
        taps: 8,
        spread_beyond_edges: false,
    }));
    eprintln!("02 post     ssr + motion + fog_froxel, same numbers as courier");

    renderer.rebuild_transient_pool();
    let (hdr_bind, depth_bind) = renderer.frame_binding_probe();
    eprintln!("03 pool     rebuilt, hdr_binding={hdr_bind} depth_binding={depth_bind}");

    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(0.9, 0.08, 0.05)));
    let camera = Camera::new(0.9, 1.0, 0.1, 100.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.8, -0.4),
        Vec3::ONE,
        6.0,
    ));
    lights.ambient = Vec3::splat(0.35);
    let cube = RenderObject {
        id: 1,
        shadow_class: redox_render::shadow::ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::from_scale_rotation_translation(
            Vec3::splat(1.4),
            Quat::IDENTITY,
            Vec3::new(0.0, 0.0, -4.0),
        ),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(Vec3::new(0.0, 0.0, -4.0), Vec3::splat(1.4)),
    };
    eprintln!(
        "04 scene    camera eye=(0,0,0) fov=0.9  cube center=(0,0,-4) scale=1.4 albedo=(0.90,0.08,0.05)"
    );
    eprintln!("05 light    dir=(0.3,-0.8,-0.4) intensity=6 ambient=0.35  frames={FRAMES}");

    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("chain_probe_present"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let objects = [cube];
    let fv = FrameView::new(&camera, &objects, &lights);
    for i in 1..=FRAMES {
        renderer.render_frame_view(&fv, &view).expect("frame");
        eprintln!("06 frame {i}/{FRAMES} submitted");
    }

    let report = renderer.frame_chain_report(Some(&target));
    eprintln!("07 readback\n{}", report.text);

    let path = std::env::temp_dir().join("redox-chain-probe.txt");
    std::fs::write(&path, &report.text).expect("write probe");
    eprintln!("08 wrote {}", path.display());

    assert_eq!(
        report.hdr_binding, "current",
        "hdr group is stale\n{}",
        report.text
    );
    assert_eq!(
        report.depth_binding, "current",
        "depth group is stale\n{}",
        report.text
    );
    assert!(
        report.hdr.center[0] > report.hdr.corner[0] + 0.1,
        "red cube is missing from HDR\n{}",
        report.text
    );
    assert!(
        (report.taa_out.center[0] - report.hdr.center[0]).abs() < 0.05,
        "TAA output does not match HDR\n{}",
        report.text
    );
    let post_peak = report.post0.max_luma.max(report.post1.max_luma);
    assert!(post_peak > 0.05, "post targets are black\n{}", report.text);
    let present = report.present.expect("present sample");
    assert!(
        present.center[0] > present.center[1],
        "tonemapped cube is not the red one\n{}",
        report.text
    );
}
