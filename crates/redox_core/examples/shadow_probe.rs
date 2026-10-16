//! **Shadow Probe** — minimal directional-shadow diagnostic.
//!
//! One bright ground plane + one vertical pole + one fixed sun.
//! Prints detailed CSM math every 2s so we can verify the shadow is
//! attached to the pole base and stretches opposite the light.
//!
//! Run: `RUST_LOG=info cargo run --example shadow_probe -p redox_core`
//!
//! ArrowLeft/Right — sun · **1** SSAO · **2** VSM · **3/4/5** SMRT perf/balanced/quality · F2/F3 debug · ESC

use std::f32::consts::FRAC_PI_4;
use std::time::Instant;

use redox_core::config::EngineConfig;
use redox_core::window::create_window;
use redox_ecs::World;
use redox_input::InputState;
use redox_math::{Mat4, Quat, Vec3, Vec4};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::systems::{RenderObject, Transform};
use redox_render::context::ShaderDebugVizMode;
use redox_render::SmrtQuality;
use redox_ui::context::UiContext;
use winit::keyboard::KeyCode;

const POLE_H: f32 = 4.0;
const POLE_BASE: Vec3 = Vec3::new(0.0, 0.0, 0.0);

#[derive(Clone, Copy)]
struct Renderable {
    mesh_index: usize,
    material_index: usize,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = EngineConfig {
        window_title: "RedOx — Shadow Probe (1 pole + plane)".to_string(),
        window_width: 1920,
        window_height: 1080,
        render_scale: 1.0,
        ..Default::default()
    };

    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    // Start with working hard CSM + SSAO. VSM is opt-in (2) — empty pages
    // used to replace CSM with fully-lit and kill cast shadows.
    ctx.enable_taa(true);
    ctx.set_ssao_enabled(true);
    ctx.set_vsm_enabled(false);
    ctx.set_smrt_quality(SmrtQuality::Performance);
    ctx.set_render_scale(1.0);
    ctx.set_exposure(1.15);
    assert!(
        !ctx.sdf_gpu.enabled && ctx.sdf_gpu.scene.atlas_dims[0] == 0,
        "SDF must stay empty for this probe (got enabled={} bricks={})",
        ctx.sdf_gpu.enabled,
        ctx.sdf_gpu.scene.atlas_dims[0]
    );
    log::info!(
        "ShadowProbe init: sdf_enabled={} bricks={} taa={} vsm={} ssao={}",
        ctx.sdf_gpu.enabled,
        ctx.sdf_gpu.scene.atlas_dims[0],
        ctx.taa_enabled,
        ctx.vsm_enabled,
        ctx.ssao_enabled
    );

    let mut ui = UiContext::new(&window, ctx.device(), ctx.surface_format());
    let mut world = World::new();
    world.insert_resource(InputState::new());

    let cube = ctx.upload_mesh(&create_cube());
    let ground_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.78, 0.76, 0.72));
        m.roughness = 0.82;
        m
    });
    let pole_mat = ctx.add_material(Material::solid(Vec3::new(0.9, 0.2, 0.15)));

    spawn(
        &mut world,
        cube,
        ground_mat,
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(30.0, 0.2, 30.0),
    );
    spawn(
        &mut world,
        cube,
        pole_mat,
        Vec3::new(0.0, POLE_H * 0.5, 0.0),
        Vec3::new(0.4, POLE_H, 0.4),
    );

    let mut sun_yaw = 0.0_f32; // sun from +X when 0
    let mut shadow_viz = false;
    let mut sources_viz = false;
    let mut use_ssao = true;
    let mut use_vsm = false;
    let mut last = Instant::now();
    let mut log_timer = 0.0_f32;
    let mut frame: u64 = 0;

    event_loop
        .run(move |event, elwt| match event {
            winit::event::Event::NewEvents(_) => {
                if let Some(input) = world.get_resource_mut::<InputState>() {
                    input.begin_frame();
                }
            }
            winit::event::Event::WindowEvent { event, .. } => {
                if let Some(input) = world.get_resource_mut::<InputState>() {
                    input.process_window_event(&event);
                }
                // Hotkeys on the KeyboardInput event itself — just_pressed on
                // RedrawRequested often misses F-keys (press+release same pump,
                // or Redraw before Key). Ignore egui "consumed" for these.
                if let winit::event::WindowEvent::KeyboardInput { event: key_ev, .. } = &event {
                    if key_ev.state == winit::event::ElementState::Pressed && !key_ev.repeat {
                        if let winit::keyboard::PhysicalKey::Code(code) = key_ev.physical_key {
                            match code {
                                KeyCode::Escape => elwt.exit(),
                                KeyCode::Digit1 => {
                                    use_ssao = !use_ssao;
                                    ctx.set_ssao_enabled(use_ssao);
                                    log::info!("SSAO={}", use_ssao);
                                }
                                KeyCode::Digit2 => {
                                    use_vsm = !use_vsm;
                                    ctx.set_vsm_enabled(use_vsm);
                                    log::info!("VSM={}", use_vsm);
                                }
                                KeyCode::Digit3 => {
                                    ctx.set_smrt_quality(SmrtQuality::Performance);
                                    log::info!("SMRT=Performance (4×5)");
                                }
                                KeyCode::Digit4 => {
                                    ctx.set_smrt_quality(SmrtQuality::Balanced);
                                    log::info!("SMRT=Balanced (6×8, default)");
                                }
                                KeyCode::Digit5 => {
                                    ctx.set_smrt_quality(SmrtQuality::Quality);
                                    log::info!("SMRT=Quality (12×10)");
                                }
                                KeyCode::F2 => {
                                    shadow_viz = !shadow_viz;
                                    if shadow_viz {
                                        sources_viz = false;
                                    }
                                    ctx.update_shader_debug_buffer(if shadow_viz {
                                        ShaderDebugVizMode::ShadowValue
                                    } else {
                                        ShaderDebugVizMode::None
                                    });
                                    log::info!("shadow_viz(grayscale)={}", shadow_viz);
                                }
                                KeyCode::F3 => {
                                    sources_viz = !sources_viz;
                                    if sources_viz {
                                        shadow_viz = false;
                                    }
                                    ctx.update_shader_debug_buffer(if sources_viz {
                                        ShaderDebugVizMode::ShadowSources
                                    } else {
                                        ShaderDebugVizMode::None
                                    });
                                    log::info!(
                                        "shadow_sources_viz={}  (Magenta=SSAO Cyan=CSM Lime=VSM Yellow=SDF)",
                                        sources_viz
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
                let _ = ui.handle_window_event(&window, &event);
                match event {
                    winit::event::WindowEvent::CloseRequested => elwt.exit(),
                    winit::event::WindowEvent::Resized(size) => {
                        ctx.resize(size.width, size.height);
                    }
                    winit::event::WindowEvent::RedrawRequested => {
                        let dt = last.elapsed().as_secs_f32().min(0.05);
                        last = Instant::now();
                        frame += 1;
                        log_timer += dt;

                        if let Some(input) = world.get_resource::<InputState>() {
                            if input.keyboard.is_pressed(KeyCode::ArrowLeft) {
                                sun_yaw -= dt * 0.9;
                            }
                            if input.keyboard.is_pressed(KeyCode::ArrowRight) {
                                sun_yaw += dt * 0.9;
                            }
                        }

                        // Fixed camera: look at pole from a known pose.
                        let look = Vec3::new(0.0, 1.0, 0.0);
                        let cam_pos = Vec3::new(8.0, 6.0, 10.0);
                        let cam_rot = Quat::from_rotation_arc(
                            Vec3::NEG_Z,
                            (look - cam_pos).normalize(),
                        );
                        let camera = Camera::new(
                            0.9,
                            ctx.config.width as f32 / ctx.config.height as f32,
                            0.1,
                            100.0,
                        );
                        ctx.camera_uniform.update(&camera, cam_pos, cam_rot);
                        ctx.update_camera_buffer();

                        // Sun: direction TOWARD the light (engine convention).
                        // yaw=0 → light from +X (shadow should stretch toward −X).
                        let sun_dir = Vec3::new(sun_yaw.cos(), 0.55, sun_yaw.sin()).normalize();
                        let mut sun =
                            DirectionalLight::new(sun_dir, Vec3::new(1.0, 0.95, 0.82), 2.1);
                        sun.soft_shadows = false;
                        sun.vsm = use_vsm;
                        let lights = LightScene {
                            directional: sun,
                            ambient: Vec3::splat(0.055),
                            points: vec![],
                            spots: vec![],
                        };
                        ctx.set_vsm_enabled(use_vsm);
                        ctx.set_ssao_enabled(use_ssao);

                        let objects = extract(&world, &ctx);
                        let fv = FrameView::new(&camera, &objects, &lights);
                        let output = ctx.surface().get_current_texture().unwrap();
                        let view = output.texture.create_view(&Default::default());
                        ctx.render_frame_view(&fv, &view).unwrap();

                        // Expected geometric shadow tip on the ground:
                        // from pole tip, cast ray opposite sun onto y=0.
                        let pole_tip = Vec3::new(0.0, POLE_H, 0.0);
                        let light_to_scene = -sun_dir; // rays travel this way
                        let t_hit = if light_to_scene.y.abs() > 1e-4 {
                            (0.0 - pole_tip.y) / light_to_scene.y
                        } else {
                            0.0
                        };
                        let expected_tip = pole_tip + light_to_scene * t_hit;
                        let expected_mid = POLE_BASE.lerp(expected_tip, 0.5);

                        if log_timer >= 2.0 {
                            log_timer = 0.0;
                            log_shadow_probe(
                                frame,
                                &ctx,
                                &camera,
                                cam_pos,
                                cam_rot,
                                sun_dir,
                                pole_tip,
                                expected_tip,
                                expected_mid,
                            );
                        }

                        // Debug lines (bool = occluded flag → color):
                        //  false → green: expected cast-shadow on ground
                        //  true  → red: toward the sun (lit side of the pole)
                        let sun_marker = pole_tip + sun_dir * 3.0;
                        ctx.set_debug_lines(vec![
                            (POLE_BASE, pole_tip, false),
                            (POLE_BASE, expected_tip, false),
                            (pole_tip, sun_marker, true),
                            (
                                expected_tip + Vec3::new(-0.3, 0.05, 0.0),
                                expected_tip + Vec3::new(0.3, 0.05, 0.0),
                                false,
                            ),
                            (
                                expected_tip + Vec3::new(0.0, 0.05, -0.3),
                                expected_tip + Vec3::new(0.0, 0.05, 0.3),
                                false,
                            ),
                        ]);

                        ui.record_frame(dt);
                        ui.begin_frame(&window);
                        egui::Area::new(egui::Id::new("probe"))
                            .anchor(egui::Align2::LEFT_TOP, [12.0, 12.0])
                            .show(&ui.egui_ctx, |ui| {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.heading("Shadow Probe");
                                    ui.label("1 pole · 1 plane · 1 sun");
                                    if ui.checkbox(&mut use_ssao, "SSAO (1)").changed() {
                                        ctx.set_ssao_enabled(use_ssao);
                                        log::info!("SSAO={}", use_ssao);
                                    }
                                    if ui.checkbox(&mut use_vsm, "VSM (2)").changed() {
                                        ctx.set_vsm_enabled(use_vsm);
                                        log::info!("VSM={}", use_vsm);
                                    }
                                    if ui.checkbox(&mut shadow_viz, "F2 grayscale shadow").changed() {
                                        if shadow_viz {
                                            sources_viz = false;
                                        }
                                        ctx.update_shader_debug_buffer(if shadow_viz {
                                            ShaderDebugVizMode::ShadowValue
                                        } else {
                                            ShaderDebugVizMode::None
                                        });
                                    }
                                    if ui
                                        .checkbox(&mut sources_viz, "F3 colour sources")
                                        .changed()
                                    {
                                        if sources_viz {
                                            shadow_viz = false;
                                        }
                                        ctx.update_shader_debug_buffer(if sources_viz {
                                            ShaderDebugVizMode::ShadowSources
                                        } else {
                                            ShaderDebugVizMode::None
                                        });
                                    }
                                    ui.label(format!(
                                        "mode: {}",
                                        if use_vsm { "VSM clipmap" } else { "hard CSM" }
                                    ));
                                    ui.label(format!(
                                        "sun_dir toward light: {:.2?}",
                                        sun_dir
                                    ));
                                    ui.label(format!(
                                        "expected tip (ground): {:.2?}",
                                        expected_tip
                                    ));
                                    if sources_viz {
                                        ui.label("MAGENTA=SSAO · CYAN=CSM · LIME=VSM · YELLOW=SDF");
                                    }
                                    ui.label("CYAN line = cast shadow · YELLOW = toward sun");
                                    ui.label("← → sun · keys or checkboxes · ESC");
                                });
                            });
                        let mut enc = ctx.device.create_command_encoder(&Default::default());
                        ui.end_frame_and_render(
                            ctx.device(),
                            ctx.queue(),
                            &mut enc,
                            &window,
                            &view,
                            egui_wgpu::ScreenDescriptor {
                                size_in_pixels: [ctx.config.width, ctx.config.height],
                                pixels_per_point: window.scale_factor() as f32,
                            },
                        );
                        ctx.queue.submit(Some(enc.finish()));
                        output.present();
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .unwrap();
}

fn log_shadow_probe(
    frame: u64,
    ctx: &RenderContext,
    camera: &Camera,
    cam_pos: Vec3,
    cam_rot: Quat,
    sun_dir: Vec3,
    pole_tip: Vec3,
    expected_tip: Vec3,
    expected_mid: Vec3,
) {
    let su = &ctx.shadow_uniform;
    let view = Camera::view_matrix(cam_pos, cam_rot);
    let proj = camera.projection_matrix();

    log::info!("──────── ShadowProbe frame={frame} ────────");
    log::info!(
        "camera near={:.3} far={:.3}  (Renderer tracks near={:.3} far={:.3})",
        camera.near,
        camera.far,
        ctx.camera_near,
        ctx.camera_far
    );
    log::info!(
        "sun_dir(toward light)={sun_dir:?}  soft_shadows_mode={}",
        ctx.light_uniform.soft_shadows
    );
    log::info!(
        "SDF enabled={} atlas_bricks={}  TAA={} VSM={}",
        ctx.sdf_gpu.enabled,
        ctx.sdf_gpu.scene.atlas_dims[0],
        ctx.taa_enabled,
        ctx.vsm_enabled
    );
    log::info!(
        "VSM dirty_pages_last_frame={}  (0 after first cached frame is expected)",
        ctx.vsm_dirty_pages_last_frame
    );
    log::info!("CSM splits={:?}", su.csm_splits);
    log::info!(
        "geometry: pole_base={POLE_BASE:?} pole_tip={pole_tip:?} expected_shadow_tip={expected_tip:?}"
    );

    // Horizontal check: tip should be opposite sun on XZ.
    let sun_xz = Vec3::new(sun_dir.x, 0.0, sun_dir.z);
    let tip_xz = Vec3::new(expected_tip.x, 0.0, expected_tip.z);
    let opposite = -sun_xz;
    let align = if tip_xz.length() > 1e-3 && opposite.length() > 1e-3 {
        tip_xz.normalize().dot(opposite.normalize())
    } else {
        0.0
    };
    log::info!(
        "expected tip vs −sun_xz align={align:.3} (want ≈ +1.0 = opposite sun on ground)"
    );

    for (name, p) in [
        ("pole_base", POLE_BASE + Vec3::new(0.0, 0.02, 0.0)),
        ("pole_tip", pole_tip),
        ("expected_mid", expected_mid + Vec3::new(0.0, 0.02, 0.0)),
        ("expected_tip", expected_tip + Vec3::new(0.0, 0.02, 0.0)),
        ("lit_control(+X)", Vec3::new(6.0, 0.02, 0.0)),
    ] {
        let view_z = -(view * Vec4::new(p.x, p.y, p.z, 1.0)).z;
        let mut cascade = 3usize;
        for i in 0..4 {
            if view_z < su.csm_splits[i] {
                cascade = i;
                break;
            }
        }
        let m = Mat4::from_cols_array_2d(&su.csm_matrices[cascade]);
        let clip = m * Vec4::new(p.x, p.y, p.z, 1.0);
        let ndc = if clip.w.abs() > 1e-6 {
            clip.truncate() / clip.w
        } else {
            Vec3::ZERO
        };
        let uv = Vec3::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
        let in_map = (0.0..=1.0).contains(&uv.x)
            && (0.0..=1.0).contains(&uv.y)
            && ndc.z <= 1.0
            && ndc.z >= 0.0;
        log::info!(
            "  {name:>16}: p={p:?} view_z={view_z:.2} cascade={cascade} uv=({:.3},{:.3}) depth={:.3} in_map={in_map}",
            uv.x,
            uv.y,
            uv.z
        );
    }

    // Cascade 0 matrix eye direction sanity: transforming a point toward the
    // sun should decrease light-space depth (closer to light / smaller z in RH ortho).
    let m0 = Mat4::from_cols_array_2d(&su.csm_matrices[0]);
    let ground = Vec3::new(0.0, 0.02, 0.0);
    let toward_sun = ground + sun_dir * 2.0;
    let z_g = (m0 * Vec4::new(ground.x, ground.y, ground.z, 1.0)).z;
    let z_s = (m0 * Vec4::new(toward_sun.x, toward_sun.y, toward_sun.z, 1.0)).z;
    log::info!(
        "cascade0 depth ground={z_g:.4} toward_sun(+2)={z_s:.4}  (toward_sun should be SMALLER if light-eye is correct)"
    );

    // Face-lighting sanity: +X face N·L vs −X. With sun_dir.x>0, +X must be
    // brighter. Side toward expected tip (−sun on XZ) must be darker.
    let n_pos_x = Vec3::X.dot(sun_dir);
    let n_neg_x = (-Vec3::X).dot(sun_dir);
    log::info!(
        "N·L face +X={n_pos_x:.3}  −X={n_neg_x:.3}  (sun-facing face should be >0; tip-side usually <0)"
    );

    // Synthetic cast test at shared UV: tip depth > pole depth ⇒ shadowed.
    let tip_p = expected_tip + Vec3::new(0.0, 0.02, 0.0);
    let view_z_tip = -(view * Vec4::new(tip_p.x, tip_p.y, tip_p.z, 1.0)).z;
    let mut cascade = 3usize;
    for i in 0..4 {
        if view_z_tip < su.csm_splits[i] {
            cascade = i;
            break;
        }
    }
    let m = Mat4::from_cols_array_2d(&su.csm_matrices[cascade]);
    let z_pole = (m * Vec4::new(pole_tip.x, pole_tip.y, pole_tip.z, 1.0)).z;
    let z_recv = (m * Vec4::new(tip_p.x, tip_p.y, tip_p.z, 1.0)).z;
    log::info!(
        "cast test cascade={cascade}: pole_depth={z_pole:.4} tip_depth={z_recv:.4} → expect SHADOW if tip>pole (LessEqual)"
    );

    let _ = (proj, FRAC_PI_4);
}

fn spawn(world: &mut World, mesh: usize, mat: usize, pos: Vec3, scale: Vec3) {
    let e = world.spawn();
    world.add_component(
        e,
        Transform {
            translation: pos,
            rotation: Quat::IDENTITY,
            scale,
        },
    );
    world.add_component(
        e,
        Renderable {
            mesh_index: mesh,
            material_index: mat,
        },
    );
}

fn extract(world: &World, ctx: &RenderContext) -> Vec<RenderObject> {
    let mut out = Vec::new();
    for e in world.all_entities() {
        let (Some(t), Some(r)) = (
            world.get_component::<Transform>(e),
            world.get_component::<Renderable>(e),
        ) else {
            continue;
        };
        let bounds = ctx
            .get_mesh_bounds(r.mesh_index)
            .map(|b| b.transform(t.matrix()))
            .unwrap_or_else(|| redox_math::Aabb {
                min: Vec3::splat(-1e9),
                max: Vec3::splat(1e9),
            });
        out.push(RenderObject {
            model_matrix: t.matrix(),
            color: [1.0; 4],
            mesh_index: r.mesh_index,
            material_index: r.material_index,
            bounds,
        });
    }
    out
}
