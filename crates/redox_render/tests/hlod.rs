//! HLOD cross-fade on the real renderer (VG-04 step 4). Skipped without a GPU.
//!
//! The detail objects are three cubes; the proxy is one bigger cube built with
//! `build_proxy` and uploaded through `upload_proxy`. Opacities are set on
//! `color[3]`, the way `apply_hlod_to_render_objects` does it.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::hlod::{HlodInput, HlodParams, build_proxy};
use redox_render::hlod_gpu::{ProxyGpu, proxy_render_object, upload_proxy};
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
/// Largest share of dark pixels allowed inside the centre of a mid-fade frame.
const MAX_DARK_SHARE: f32 = 0.02;
/// Largest mean-brightness step between consecutive fade frames, in 0..=1 units.
const MAX_BRIGHTNESS_STEP: f32 = 0.05;
/// A pixel whose RGB sum is below this counts as a black hole.
const DARK_SUM: u32 = 24;
const FADE_STEPS: usize = 10;

struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    camera: Camera,
    lights: LightScene,
    details: Vec<RenderObject>,
    proxy: ProxyGpu,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(true);
    let cube = create_cube();
    let cube_mesh = renderer.upload_mesh(&cube);
    let cube_material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.5, 0.3)));
    let details = [-1.0_f32, 0.0, 1.0]
        .iter()
        .enumerate()
        .map(|(i, x)| RenderObject {
            id: i as u32,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(
                Vec3::splat(0.8),
                Quat::IDENTITY,
                Vec3::new(*x, 0.0, -10.0),
            ),
            color: [1.0; 4],
            mesh_index: cube_mesh,
            material_index: cube_material,
            bounds: Aabb::from_center_size(Vec3::new(*x, 0.0, -10.0), Vec3::ONE),
        })
        .collect();

    // The proxy is one box stretched over the three detail cubes.
    let positions: Vec<[f32; 3]> = cube.vertices.iter().map(|v| v.position).collect();
    let transform = Mat4::from_scale_rotation_translation(
        Vec3::new(3.0, 1.0, 1.0),
        Quat::IDENTITY,
        Vec3::new(0.0, 0.0, -10.0),
    );
    let input = HlodInput {
        positions: &positions,
        indices: &cube.indices,
        transform,
        color: [0.8, 0.5, 0.3],
        vertex_colors: None,
    };
    let params = HlodParams {
        target_triangles: 2000,
        max_error: 0.05,
        weld_epsilon: 1e-4,
    };
    let proxy_cpu = build_proxy(&[input], &params).unwrap();
    let proxy = upload_proxy(&mut renderer, &proxy_cpu);

    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hlod_target"),
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
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, -0.3, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.6);
    Some(Rig {
        renderer,
        target,
        view,
        camera,
        lights,
        details,
        proxy,
    })
}

impl Rig {
    /// Frame for a fade position `t` in `0..=1`: details at `1 - t`, proxy at `t`.
    fn objects_at(&self, t: f32) -> Vec<RenderObject> {
        let mut objects: Vec<RenderObject> = self
            .details
            .iter()
            .map(|detail| {
                let mut object = detail.clone();
                object.color[3] = 1.0 - t;
                object
            })
            .filter(|object| object.color[3] > 0.0)
            .collect();
        if t > 0.0 {
            objects.push(proxy_render_object(&self.proxy, 1000, t));
        }
        objects
    }

    /// Renders one frame and returns it as RGBA bytes.
    fn frame(&mut self, objects: &[RenderObject]) -> Vec<u8> {
        let view = FrameView::new(&self.camera, objects, &self.lights);
        self.renderer
            .render_frame_view(&view, &self.view)
            .expect("frame");
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        read_texture_ktx2(
            &self.renderer.device,
            &self.renderer.queue,
            &self.target,
            false,
        )
        .unwrap()
        .levels
        .swap_remove(0)
    }
}

/// Share of dark pixels in the centre half of the frame.
fn dark_share_in_centre(frame: &[u8]) -> f32 {
    let (lo, hi) = (SIZE / 4, 3 * SIZE / 4);
    let mut dark = 0;
    let mut total = 0;
    for y in lo..hi {
        for x in lo..hi {
            let at = ((y * SIZE + x) * 4) as usize;
            let sum = u32::from(frame[at]) + u32::from(frame[at + 1]) + u32::from(frame[at + 2]);
            total += 1;
            if sum < DARK_SUM {
                dark += 1;
            }
        }
    }
    dark as f32 / total as f32
}

/// Mean brightness of a frame in `0..=1`.
fn mean_brightness(frame: &[u8]) -> f32 {
    let sum: u64 = frame
        .chunks_exact(4)
        .map(|px| u64::from(px[0]) + u64::from(px[1]) + u64::from(px[2]))
        .sum();
    sum as f32 / (frame.len() / 4 * 3) as f32 / 255.0
}

#[test]
fn a_mid_fade_frame_shows_both_without_black_holes() {
    let Some(mut rig) = rig() else {
        return;
    };
    let objects = rig.objects_at(0.5);
    let frame = rig.frame(&objects);
    let share = dark_share_in_centre(&frame);
    assert!(
        share <= MAX_DARK_SHARE,
        "dark share {share} in the centre of a mid-fade frame"
    );
}

/// Two cubes of different flat colours (red and blue, not touching) merged into one proxy.
/// Returns the CPU proxy; its mean colour is the expected cell colour.
fn red_and_blue_proxy() -> redox_render::hlod::HlodProxy {
    let cube = create_cube();
    let positions: Vec<[f32; 3]> = cube.vertices.iter().map(|v| v.position).collect();
    let red = HlodInput {
        positions: &positions,
        indices: &cube.indices,
        transform: Mat4::from_translation(Vec3::new(-1.0, 0.0, -10.0)),
        color: [1.0, 0.0, 0.0],
        vertex_colors: None,
    };
    let blue = HlodInput {
        transform: Mat4::from_translation(Vec3::new(1.0, 0.0, -10.0)),
        color: [0.0, 0.0, 1.0],
        ..red
    };
    let params = HlodParams {
        target_triangles: 2000,
        max_error: 0.05,
        weld_epsilon: 1e-4,
    };
    build_proxy(&[red, blue], &params).unwrap()
}

#[test]
fn the_proxy_colour_is_the_mean_colour_of_its_sources() {
    let proxy = red_and_blue_proxy();
    let mean = proxy.mean_colour();
    // Equal numbers of red and blue vertices: the mean is half red, half blue, within 1/255.
    for (got, want) in mean.iter().zip([0.5, 0.0, 0.5]) {
        assert!((got - want).abs() <= 1.0 / 255.0, "mean {mean:?}");
    }
}

#[test]
fn an_uploaded_proxy_is_drawn_with_its_mean_colour() {
    let Some(adapter) = common::adapter() else {
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    let proxy_cpu = red_and_blue_proxy();
    let proxy = upload_proxy(&mut renderer, &proxy_cpu);
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hlod_upload_target"),
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
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, -0.3, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.6);
    let mut render = |objects: &[_]| {
        let frame_view = FrameView::new(&camera, objects, &lights);
        renderer
            .render_frame_view(&frame_view, &view)
            .expect("frame");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)
            .unwrap()
            .levels
            .swap_remove(0)
    };
    let background = render(&[]);
    let frame = render(&[proxy_render_object(&proxy, 1, 1.0)]);
    // Pixels the proxy changed: its two cubes sit left and right of the centre. Their mean colour
    // is a red-blue mix, so green must be the smallest channel on average.
    let (mut covered, mut sum) = (0u32, [0u64; 3]);
    for (px, bg) in frame.chunks_exact(4).zip(background.chunks_exact(4)) {
        if px[..3]
            .iter()
            .zip(&bg[..3])
            .any(|(a, b)| a.abs_diff(*b) > 8)
        {
            covered += 1;
            for c in 0..3 {
                sum[c] += u64::from(px[c]);
            }
        }
    }
    assert!(covered > 20, "the proxy covers only {covered} pixels");
    let n = u64::from(covered);
    let (r, g, b) = (sum[0] / n, sum[1] / n, sum[2] / n);
    assert!(
        g < r && g < b,
        "mean colour of the proxy ({r}, {g}, {b}) is not red and blue"
    );
}

#[test]
fn no_flash_at_the_switch() {
    let Some(mut rig) = rig() else {
        return;
    };
    let brightness: Vec<f32> = (0..=FADE_STEPS)
        .map(|step| {
            let t = step as f32 / FADE_STEPS as f32;
            let objects = rig.objects_at(t);
            mean_brightness(&rig.frame(&objects))
        })
        .collect();
    for pair in brightness.windows(2) {
        let step = (pair[1] - pair[0]).abs();
        assert!(
            step <= MAX_BRIGHTNESS_STEP,
            "brightness jumped by {step} between fade steps: {brightness:?}"
        );
    }
}
