//! Horror demo: dark scene with objects and figures; lamp turns on by key press.
//! No physics — static scene, same PBR pipeline as simple_game.

use std::sync::Arc;
use std::time::Instant;
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

use redox_ecs::world::World;
use redox_math::{Mat4, Quat, Transform, Vec3};

use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::light::{DirectionalLight, LightUniform, PointLight};
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::systems::{
    extract_render_objects, sync_assets_to_render, MaterialHandle, MeshHandle,
};
use redox_render::{asset_types::MaterialData, asset_types::MeshData};
use redox_asset::{AssetManager, Handle};

use redox_input::state::InputState;
use egui::Color32;
use redox_ui::context::UiContext;

// --- Constants ---
const LAMP_INTENSITY_ON: f32 = 10.0;
const LAMP_RADIUS: f32 = 12.0;

/// All mesh/material handles for sync.
struct GameAssets {
    mesh_handles: Vec<Handle<MeshData>>,
    material_handles: Vec<Handle<MaterialData>>,
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    pollster::block_on(run());
}

async fn run() {
    log::info!("Horror Demo — dark scene, lamp toggle");

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RedOx Engine — Horror Demo")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .unwrap(),
    );

    let mut render_ctx = RenderContext::new(window.clone()).await;
    let mut ui_ctx = UiContext::new(&window, render_ctx.device(), render_ctx.surface_format());
    let mut input_state = InputState::new();

    let mut world = World::new();

    // --- Assets ---
    let mut asset_manager = AssetManager::new(".");
    let mesh_cube = MeshHandle(asset_manager.insert(create_cube()));
    let mesh_sphere = MeshHandle(asset_manager.insert(create_sphere(0.4, 20)));

    let mat_floor = MaterialHandle(asset_manager.insert(
        MaterialData::solid(Vec3::new(0.04, 0.04, 0.06))
            .metallic(0.0)
            .roughness(0.9),
    ));
    let mat_table = MaterialHandle(asset_manager.insert(
        MaterialData::solid(Vec3::new(0.08, 0.05, 0.03))
            .metallic(0.0)
            .roughness(0.8),
    ));
    let mat_figure_dark = MaterialHandle(asset_manager.insert(
        MaterialData::solid(Vec3::new(0.12, 0.08, 0.1))
            .metallic(0.1)
            .roughness(0.7),
    ));
    let mat_figure_light = MaterialHandle(asset_manager.insert(
        MaterialData::solid(Vec3::new(0.2, 0.18, 0.22))
            .metallic(0.2)
            .roughness(0.6),
    ));

    let game_assets = GameAssets {
        mesh_handles: vec![mesh_cube.0, mesh_sphere.0],
        material_handles: vec![mat_floor.0, mat_table.0, mat_figure_dark.0, mat_figure_light.0],
    };
    world.insert_resource(game_assets);

    // --- IBL (optional) ---
    let hdr_path = "assets/skybox.hdr";
    if std::path::Path::new(hdr_path).exists() {
        if let Ok(hdr_bytes) = std::fs::read(hdr_path) {
            if let Ok(hdr_tex) = redox_render::resource::texture::Texture::from_hdr_bytes(
                render_ctx.device(),
                render_ctx.queue(),
                &hdr_bytes,
                "Skybox HDR",
            ) {
                render_ctx.set_environment(&hdr_tex);
            }
        }
    }

    // --- Scene: dark floor ---
    let floor_entity = world.spawn();
    world.add_component(
        floor_entity,
        Transform {
            translation: Vec3::new(0.0, -0.5, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(20.0, 1.0, 20.0),
        },
    );
    world.add_component(floor_entity, mesh_cube.clone());
    world.add_component(floor_entity, mat_floor.clone());

    // --- Table (flat box) ---
    let table_entity = world.spawn();
    world.add_component(
        table_entity,
        Transform {
            translation: Vec3::new(0.0, 0.4, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(4.0, 0.15, 2.0),
        },
    );
    world.add_component(table_entity, mesh_cube.clone());
    world.add_component(table_entity, mat_table.clone());

    // --- Figures on the table (cubes and spheres) ---
    let figure_positions = [
        (Vec3::new(-0.8, 0.55, 0.2), true),   // cube
        (Vec3::new(0.0, 0.6, 0.0), false),  // sphere
        (Vec3::new(0.7, 0.55, -0.15), true),
        (Vec3::new(-0.3, 0.65, -0.3), false),
        (Vec3::new(0.4, 0.55, 0.25), true),
    ];
    for (pos, is_cube) in figure_positions {
        let e = world.spawn();
        world.add_component(
            e,
            Transform {
                translation: pos,
                rotation: Quat::IDENTITY,
                scale: Vec3::new(0.2, 0.2, 0.2),
            },
        );
        world.add_component(e, if is_cube { mesh_cube.clone() } else { mesh_sphere.clone() });
        world.add_component(
            e,
            if pos.x > 0.0 { mat_figure_light.clone() } else { mat_figure_dark.clone() },
        );
    }

    // --- Directional light (very dim, almost night) ---
    let light_entity = world.spawn();
    world.add_component(
        light_entity,
        DirectionalLight {
            direction: Vec3::new(0.2, -1.0, 0.3).normalize(),
            color: Vec3::new(0.2, 0.18, 0.25),
            intensity: 0.3,
        },
    );

    // --- Lamp (point light above the table); no mesh — only light ---
    let lamp_pos = Vec3::new(0.0, 2.2, 0.5);
    let lamp_entity = world.spawn();
    world.add_component(
        lamp_entity,
        Transform {
            translation: lamp_pos,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.add_component(
        lamp_entity,
        PointLight::new(
            lamp_pos,
            Vec3::new(0.95, 0.9, 0.75),
            0.0, // off by default
            LAMP_RADIUS,
        ),
    );

    // --- Camera (look at table from front) ---
    let camera_entity = world.spawn();
    let cam_pos = Vec3::new(0.0, 1.2, 5.0);
    let view_mat = Mat4::look_at_rh(cam_pos, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
    world.add_component(
        camera_entity,
        Camera {
            fov_y: 50.0_f32.to_radians(),
            near: 0.1,
            far: 500.0,
            aspect_ratio: 1280.0 / 720.0,
        },
    );
    world.add_component(
        camera_entity,
        Transform {
            translation: cam_pos,
            rotation: Quat::from_mat4(&view_mat.inverse()),
            scale: Vec3::ONE,
        },
    );

    let mut start_time = Instant::now();
    let mut last_dt = 1.0 / 60.0;

    #[allow(deprecated)]
    let _ = event_loop.run(move |event, control_flow| {
        if let Event::WindowEvent { event: ref we, .. } = event {
            if ui_ctx.handle_window_event(&window, we) {
                return;
            }
            input_state.process_window_event(we);
        }

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => control_flow.exit(),

            Event::WindowEvent {
                event: WindowEvent::Resized(physical_size),
                ..
            } => {
                render_ctx.resize(physical_size.width, physical_size.height);
                if let Some(cam) = world.get_component_mut::<Camera>(camera_entity) {
                    cam.aspect_ratio = physical_size.width as f32 / physical_size.height as f32;
                }
            }

            Event::AboutToWait => {
                let now = Instant::now();
                last_dt = now.duration_since(start_time).as_secs_f32();
                start_time = now;

                use winit::keyboard::KeyCode;
                if input_state.keyboard.just_pressed(KeyCode::KeyL) {
                    if let Some(pl) = world.get_component_mut::<PointLight>(lamp_entity) {
                        let currently_on = pl.intensity > 0.0;
                        pl.intensity = if currently_on { 0.0 } else { LAMP_INTENSITY_ON };
                        log::info!("Lamp: {}", if currently_on { "OFF" } else { "ON" });
                    }
                }

                input_state.begin_frame();
                window.request_redraw();
            }

            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                asset_manager.update(&mut world);
                if let Some(assets) = world.get_resource::<GameAssets>() {
                    sync_assets_to_render(
                        &mut render_ctx,
                        &asset_manager,
                        &assets.mesh_handles,
                        &[],
                        &assets.material_handles,
                    );
                }

                if let Some(cam) = world.get_component::<Camera>(camera_entity) {
                    let tf = world.get_component::<Transform>(camera_entity).unwrap();
                    let view = Mat4::from_quat(tf.rotation).inverse()
                        * Mat4::from_translation(-tf.translation);
                    let proj = cam.projection_matrix();
                    render_ctx.camera_uniform.view_proj = (proj * view).to_cols_array_2d();
                    render_ctx.camera_uniform.camera_pos =
                        [tf.translation.x, tf.translation.y, tf.translation.z, 1.0];
                    render_ctx.update_camera_buffer();
                }

                if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                    let mut light_u = LightUniform {
                        dir_direction: [
                            light.direction.x,
                            light.direction.y,
                            light.direction.z,
                            0.0,
                        ],
                        dir_color: [light.color.x, light.color.y, light.color.z, light.intensity],
                        ambient: [0.03, 0.03, 0.05, 1.0],
                        ..Default::default()
                    };
                    for e in world.all_entities() {
                        if let Some(pl) = world.get_component::<PointLight>(e) {
                            let mut pl_cloned = pl.clone();
                            if let Some(tf) = world.get_component::<Transform>(e) {
                                pl_cloned.position = tf.translation;
                            }
                            light_u.add_point_light(&pl_cloned);
                        }
                    }
                    render_ctx.update_light_buffer(&light_u);
                    
                    // Update cluster lights for better performance with many lights
                    if let Some(cam) = world.get_component::<Camera>(camera_entity) {
                        let mut point_lights = Vec::new();
                        for e in world.all_entities() {
                            if let Some(pl) = world.get_component::<PointLight>(e) {
                                let mut pl_cloned = pl.clone();
                                if let Some(tf) = world.get_component::<Transform>(e) {
                                    pl_cloned.position = tf.translation;
                                }
                                point_lights.push(pl_cloned);
                            }
                        }
                        render_ctx.update_cluster_lights(&point_lights, &cam);
                    }
                }

                let render_objects = extract_render_objects(&world, &render_ctx);

                let output = render_ctx.surface().get_current_texture().unwrap();
                let surface_view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = render_ctx
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Horror Encoder"),
                    });

                let shadow_matrix = {
                    if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                        let size = 25.0;
                        let proj = redox_math::orthographic(-size, size, -size, size, -60.0, 60.0);
                        let view = redox_math::look_at(
                            light.direction * -25.0,
                            redox_math::Vec3::ZERO,
                            redox_math::Vec3::Y,
                        );
                        proj * view
                    } else {
                        redox_math::Mat4::IDENTITY
                    }
                };

                if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                    let mut light_u = LightUniform {
                        dir_direction: [
                            light.direction.x, light.direction.y, light.direction.z, 0.0,
                        ],
                        dir_color: [light.color.x, light.color.y, light.color.z, light.intensity],
                        ambient: [0.03, 0.03, 0.05, 1.0],
                        shadow_view_proj: shadow_matrix.to_cols_array_2d(),
                        ..Default::default()
                    };
                    for e in world.all_entities() {
                        if let Some(pl) = world.get_component::<PointLight>(e) {
                            let mut pl_cloned = pl.clone();
                            if let Some(tf) = world.get_component::<Transform>(e) {
                                pl_cloned.position = tf.translation;
                            }
                            light_u.add_point_light(&pl_cloned);
                        }
                    }
                    render_ctx.update_light_buffer(&light_u);
                    
                    // Update cluster lights for better performance with many lights
                    if let Some(cam) = world.get_component::<Camera>(camera_entity) {
                        let mut point_lights = Vec::new();
                        for e in world.all_entities() {
                            if let Some(pl) = world.get_component::<PointLight>(e) {
                                let mut pl_cloned = pl.clone();
                                if let Some(tf) = world.get_component::<Transform>(e) {
                                    pl_cloned.position = tf.translation;
                                }
                                point_lights.push(pl_cloned);
                            }
                        }
                        render_ctx.update_cluster_lights(&point_lights, &cam);
                    }
                }

                render_ctx.update_model_buffer(&render_objects);

                {
                    let shadow_view_proj_buffer =
                        redox_render::resource::buffer::create_uniform_buffer(
                            render_ctx.device(),
                            "shadow_matrix",
                            bytemuck::bytes_of(&shadow_matrix.to_cols_array_2d()),
                        );
                    let shadow_bg = render_ctx.device().create_bind_group(
                        &wgpu::BindGroupDescriptor {
                            label: Some("shadow_bg"),
                            layout: &render_ctx.shadow_pass.bind_group_layout,
                            entries: &[wgpu::BindGroupEntry {
                                binding: 0,
                                resource: shadow_view_proj_buffer.as_entire_binding(),
                            }],
                        },
                    );
                    let mut s_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shadow_pass"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.shadow_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    s_pass.set_pipeline(&render_ctx.shadow_pass.pipeline);
                    s_pass.set_bind_group(0, &shadow_bg, &[]);
                    s_pass.set_bind_group(1, &render_ctx.shadow_model_bind_group, &[]);
                    for (i, obj) in render_objects.iter().enumerate() {
                        if let Some(gpu_mesh) = render_ctx.meshes.get(obj.mesh_index) {
                            s_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                            s_pass.set_index_buffer(
                                gpu_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            s_pass.draw_indexed(
                                0..gpu_mesh.index_count,
                                0,
                                (i as u32)..(i as u32 + 1),
                            );
                        }
                    }
                }

                {
                    let mut normal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("normal_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.normal_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    normal_pass.set_pipeline(&render_ctx.normal_pass.pipeline);
                    normal_pass.set_bind_group(0, &render_ctx.normal_bind_group, &[]);
                    for (i, obj) in render_objects.iter().enumerate() {
                        if let Some(gpu_mesh) = render_ctx.meshes.get(obj.mesh_index) {
                            normal_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                            normal_pass.set_index_buffer(
                                gpu_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            normal_pass.draw_indexed(
                                0..gpu_mesh.index_count,
                                0,
                                (i as u32)..(i as u32 + 1),
                            );
                        }
                    }
                }

                {
                    let mut ssao_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("ssao_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.ssao_raw_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 1.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    ssao_pass.set_pipeline(&render_ctx.ssao_pass.pipeline);
                    ssao_pass.set_bind_group(0, &render_ctx.ssao_bind_group, &[]);
                    ssao_pass.draw(0..3, 0..1);
                }

                {
                    let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("ssao_blur_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.ssao_blurred_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 1.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    blur_pass.set_pipeline(&render_ctx.ssao_pass.blur_pipeline);
                    blur_pass.set_bind_group(0, &render_ctx.ssao_blur_bind_group, &[]);
                    blur_pass.draw(0..3, 0..1);
                }

                {
                    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("scene_pass_hdr"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.hdr_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.02,
                                    g: 0.02,
                                    b: 0.04,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    render_ctx.record_draw(&mut rpass, &render_objects);
                }

                {
                    let mut final_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("final_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &surface_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    final_pass.set_pipeline(&render_ctx.tone_mapping_pipeline);
                    final_pass.set_bind_group(0, &render_ctx.tone_mapping_bind_group, &[]);
                    final_pass.draw(0..3, 0..1);
                }

                ui_ctx.record_frame(last_dt);
                ui_ctx.begin_frame(&window);

                let lamp_on = world
                    .get_component::<PointLight>(lamp_entity)
                    .map(|pl| pl.intensity > 0.0)
                    .unwrap_or(false);

                egui::Window::new("Horror Demo")
                    .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
                    .show(&ui_ctx.egui_ctx, |ui| {
                        ui.heading("Dark scene · Lamp");
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Lamp:");
                            if lamp_on {
                                ui.label(egui::RichText::new("ON").color(Color32::from_rgb(255, 220, 100)));
                            } else {
                                ui.label(egui::RichText::new("OFF").color(Color32::GRAY));
                            }
                        });
                        ui.separator();
                        ui.label("L — toggle lamp");
                    });

                ui_ctx.draw_debug(&world);
                let screen_desc = egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [render_ctx.config().width, render_ctx.config().height],
                    pixels_per_point: window.scale_factor() as f32,
                };
                ui_ctx.end_frame_and_render(
                    render_ctx.device(),
                    render_ctx.queue(),
                    &mut encoder,
                    &window,
                    &surface_view,
                    screen_desc,
                );
                render_ctx.queue().submit(std::iter::once(encoder.finish()));
                output.present();
            }

            _ => {}
        }
    });
}
