//! Phase-1 showcase: PBR + VSM directional shadows + multi point-light shadows
//! + SSAO + egui stats. Orbiting camera proves the VSM clipmap tracks the viewer.
//!
//! Run: cargo run --example showcase -p redox_core

// Demo code: panicking on setup failure is acceptable here.
#![allow(clippy::unwrap_used)]
use std::time::Instant;

use redox_core::config::EngineConfig;
use redox_core::window::RunClosure;
use redox_core::window::create_window;
use redox_ecs::World;
use redox_input::InputState;
use redox_math::{Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{LightScene, PointLight};
use redox_render::material::Material;
use redox_render::mesh::primitive;
use redox_render::systems::{RenderObject, Transform, prepare_vg_frame};
use redox_render::virtual_geometry::VirtualMesh;
use redox_ui::context::UiContext;
use winit::keyboard::KeyCode;

/// Marker linking an entity to GPU slots uploaded at startup.
#[derive(Clone, Copy)]
struct Renderable {
    mesh_index: usize,
    material_index: usize,
}

fn main() {
    env_logger::init();
    let config = EngineConfig {
        window_title: "RedOx Showcase — Phase 1".to_string(),
        ..Default::default()
    };

    let (event_loop, window_arc) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window_arc.clone(),
        config.wgpu_present_mode(),
    ));
    ctx.set_lighting_mode(config.lighting_mode.into());
    ctx.set_render_scale(config.render_scale);
    let mut ui = UiContext::new(&window_arc, ctx.device(), ctx.surface_format());

    let mut world = World::new();
    world.insert_resource(InputState::new());

    // ── Scene ────────────────────────────────────────────────────────────────
    let ground_mesh = ctx.upload_mesh(&primitive::create_cube());
    let box_mesh = ctx.upload_mesh(&primitive::create_cube());

    let ground_mat = ctx.add_material(Material::solid(Vec3::new(0.35, 0.32, 0.30)));
    let red_mat = ctx.add_material(Material::solid(Vec3::new(0.85, 0.15, 0.12)));
    let green_mat = ctx.add_material(Material::solid(Vec3::new(0.15, 0.75, 0.25)));
    let blue_mat = ctx.add_material(Material::solid(Vec3::new(0.2, 0.4, 0.9)));

    spawn(
        &mut world,
        ground_mesh,
        ground_mat,
        Vec3::ZERO,
        Vec3::new(24.0, 0.25, 24.0),
    );
    // Non-uniform scale exercises the inverse-transpose normal matrix.
    spawn(
        &mut world,
        box_mesh,
        red_mat,
        Vec3::new(-3.0, 1.1, 0.0),
        Vec3::new(2.0, 0.5, 2.0),
    );
    spawn(
        &mut world,
        box_mesh,
        green_mat,
        Vec3::new(0.5, 1.5, -1.5),
        Vec3::new(1.2, 3.0, 1.2),
    );
    spawn(
        &mut world,
        box_mesh,
        blue_mat,
        Vec3::new(3.2, 0.9, 1.8),
        Vec3::ONE,
    );
    spawn(
        &mut world,
        box_mesh,
        green_mat,
        Vec3::new(2.2, 0.45, -3.4),
        Vec3::splat(0.9),
    );

    // Phase-3 VG object: visibility buffer + PBR resolve path.
    let vg_asset = ctx
        .register_vg_mesh(&primitive::create_cube(), blue_mat as u32)
        .expect("VG asset registration");
    let vg_entity = world.spawn();
    world.add_component(
        vg_entity,
        Transform {
            translation: Vec3::new(-1.5, 1.2, 2.5),
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(1.8),
        },
    );
    world.add_component(vg_entity, VirtualMesh::new(vg_asset));

    // Eight shadow-casting point lights (Phase-2 multi-caster atlas gate).
    let mut points = Vec::new();
    for i in 0..8 {
        let angle = i as f32 / 8.0 * std::f32::consts::TAU;
        let mut l = PointLight::new(
            Vec3::new(angle.cos() * 5.5, 3.0, angle.sin() * 5.5),
            match i % 4 {
                0 => Vec3::new(1.0, 0.45, 0.25),
                1 => Vec3::new(0.3, 0.55, 1.0),
                2 => Vec3::new(0.35, 1.0, 0.45),
                _ => Vec3::new(0.95, 0.9, 1.0),
            },
            16.0,
            10.0,
        );
        l.cast_shadows = true;
        points.push(l);
    }

    let mut last = Instant::now();
    let mut orbit_t: f32 = 0.0;
    let mut use_vsm = false;
    let mut use_scatter_light = false;
    let mut render_scale = config.render_scale;

    event_loop
        .run_closure(move |event, elwt| match event {
            winit::event::Event::NewEvents(_) => {
                if let Some(input) = world.get_resource_mut::<InputState>() {
                    input.begin_frame();
                }
            }
            winit::event::Event::WindowEvent { event, .. } => {
                if let Some(input) = world.get_resource_mut::<InputState>() {
                    input.process_window_event(&event);
                }
                if ui.handle_window_event(&window_arc, &event) {
                    return;
                }
                match event {
                    winit::event::WindowEvent::CloseRequested => elwt.exit(),
                    winit::event::WindowEvent::Resized(size) => {
                        ctx.resize(size.width, size.height);
                    }
                    winit::event::WindowEvent::RedrawRequested => {
                        let dt = last.elapsed().as_secs_f32().min(0.05);
                        last = Instant::now();
                        orbit_t += dt * 0.35;

                        if let Some(input) = world.get_resource::<InputState>()
                            && input.keyboard.is_pressed(KeyCode::Escape)
                        {
                            elwt.exit();
                        }

                        // Orbiting camera — VSM clipmap must follow the real view.
                        let radius = 13.0;
                        let cam_pos =
                            Vec3::new(orbit_t.sin() * radius, 6.5, orbit_t.cos() * radius);
                        let camera = Camera::new(
                            0.9,
                            ctx.config.width as f32 / ctx.config.height as f32,
                            0.1,
                            200.0,
                        );
                        ctx.camera_uniform.update(&camera, cam_pos, Quat::IDENTITY);
                        ctx.update_camera_buffer();

                        // Directional sun (PCSS soft shadows ON) + 8 point casters.
                        let mut sun = redox_render::light::DirectionalLight::new(
                            Vec3::new(-0.55, 1.0, 0.35),
                            Vec3::new(1.0, 0.96, 0.9),
                            1.4,
                        );
                        sun.soft_shadows = !use_vsm;
                        sun.vsm = use_vsm;
                        let scene = LightScene {
                            directional: sun,
                            ambient: Vec3::splat(0.12),
                            points: points.clone(),
                            spots: vec![],
                        };
                        ctx.set_vsm_enabled(use_vsm);
                        ctx.set_lighting_mode(if use_scatter_light {
                            redox_render::LightingMode::ScatterLight
                        } else {
                            redox_render::LightingMode::Clustered
                        });
                        if (ctx.render_scale - render_scale).abs() > 0.001 {
                            ctx.set_render_scale(render_scale);
                        }
                        ctx.update_camera_buffer();

                        prepare_vg_frame(&mut world, &mut ctx);

                        let objects = extract_showcase_objects(&world, &ctx);
                        let fv = FrameView::new(&camera, &objects, &scene);

                        let output = redox_render::acquire_surface_texture(ctx.surface()).unwrap();
                        let view = output.texture.create_view(&Default::default());
                        ctx.render_frame_view(&fv, &view).unwrap();

                        // ── egui overlay on the same surface texture ──
                        ui.record_frame(dt);
                        ui.begin_frame(&window_arc);
                        egui::Area::new(egui::Id::new("stats"))
                            .anchor(egui::Align2::LEFT_TOP, [12.0, 12.0])
                            .show(&ui.egui_ctx, |ui| {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.monospace(format!("fps: {:5.1}", 1.0 / dt.max(1e-4)));
                                    ui.monospace(format!(
                                        "entities: {}",
                                        world.all_entities().count()
                                    ));
                                    ui.monospace(format!("objects: {}", objects.len()));
                                    ui.checkbox(&mut use_vsm, "VSM directional shadows");
                                    ui.checkbox(&mut use_scatter_light, "ScatterLight path");
                                    ui.add(
                                        egui::Slider::new(&mut render_scale, 0.5..=1.0)
                                            .text("TSR render scale")
                                            .step_by(0.05),
                                    );
                                    ui.monospace(format!(
                                        "internal: {}×{}  TSR: {}",
                                        ctx.internal_width,
                                        ctx.internal_height,
                                        ctx.uses_tsr()
                                    ));
                                    ui.monospace(format!(
                                        "VSM dirty pages: {}",
                                        ctx.vsm_dirty_pages_last_frame
                                    ));
                                    ui.monospace(format!(
                                        "shadow casters: {}",
                                        points.iter().filter(|p| p.cast_shadows).count()
                                    ));
                                    if let Some(vg) = &ctx.vg_system {
                                        let s = vg.stats();
                                        ui.monospace(format!(
                                            "VG meshlets: {}/{}",
                                            s.visible_meshlets, s.total_meshlets
                                        ));
                                        ui.monospace(format!(
                                            "VG tris: {}/{}",
                                            s.visible_triangles, s.total_triangles
                                        ));
                                    }
                                    ui.monospace("VSM toggle; 8 point atlas shadows");
                                    ui.monospace("ESC exits");
                                });
                            });
                        ui.draw_debug(&mut world);
                        let mut encoder = ctx.device.create_command_encoder(&Default::default());
                        ui.end_frame_and_render(
                            ctx.device(),
                            ctx.queue(),
                            &mut encoder,
                            &window_arc,
                            &view,
                            egui_wgpu::ScreenDescriptor {
                                size_in_pixels: [ctx.config.width, ctx.config.height],
                                pixels_per_point: window_arc.scale_factor() as f32,
                            },
                        );
                        ctx.queue.submit(Some(encoder.finish()));
                        ctx.queue().present(output);
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => window_arc.request_redraw(),
            _ => {}
        })
        .unwrap();
}

fn extract_showcase_objects(world: &World, ctx: &RenderContext) -> Vec<RenderObject> {
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
                min: Vec3::splat(-1.0e9),
                max: Vec3::splat(1.0e9),
            });
        out.push(RenderObject {
            model_matrix: t.matrix(),
            color: [1.0, 1.0, 1.0, 1.0],
            mesh_index: r.mesh_index,
            material_index: r.material_index,
            bounds,
        });
    }
    out
}

fn spawn(world: &mut World, mesh: usize, material: usize, pos: Vec3, scale: Vec3) {
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
            material_index: material,
        },
    );
}
