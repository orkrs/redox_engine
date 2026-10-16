//! Per-frame work: input-driven camera, deferred editor requests, viewport render,
//! egui pass and presentation.

use super::EditorApp;
use crate::commands::{collect_transforms, resolve_entity};
use crate::render_host::{RenderHost, SharedGpu};
use redox_editor_core::{
    ViewportLayout, ViewportLinesPass, ViewportShading, WorldSession, build_editor_overlay_lines,
    egui_to_viewport_coords, ghost_cube_lines, ghost_move_line, pick_in_quad_pane, pick_unit_cubes,
    place_on_surface, scatter_poses,
};
use redox_editor_ui::{AssetKind, EditorMeshProxy, OutlinerAction};
use redox_math::{Transform, Vec3};
use std::time::Instant;

impl EditorApp {
    pub(super) fn on_redraw(&mut self) {
        self.tick_camera_and_background();
        self.drain_asset_spawn_requests();
        self.handle_timetravel_status();
        self.ensure_gpu();
        self.handle_world_status_requests();
        self.handle_outliner_actions();

        let entities = collect_transforms(&self.world.lock());
        let selected_id = self.selection.lock().primary.map(|e| e.id());
        let shading = *self.ui_ctx.shading.lock();
        let layout = *self.ui_ctx.layout.lock();

        self.resize_viewport_target();
        self.render_viewport(&entities, selected_id, shading, layout);
        let full_output = self.build_ui();
        self.pick_on_click(&entities);
        {
            let mut world_guard = self.world.lock();
            let registry_guard = self.registry.lock();
            let mut undo_guard = self.undo_stack.lock();
            undo_guard.handle_hotkeys(&self.egui_ctx, &mut world_guard, &registry_guard);
        }
        self.present(full_output);
    }

    fn tick_camera_and_background(&mut self) {
        let dt = self.last_frame.elapsed().as_secs_f32().max(1e-4);
        self.last_frame = Instant::now();
        self.fps_accum += dt;
        self.fps_frames += 1;
        if self.fps_accum >= 0.25 {
            let fps = self.fps_frames as f32 / self.fps_accum;
            *self.ui_ctx.fps.lock() = fps;
            *self.ui_ctx.frame_ms.lock() = 1000.0 / fps.max(1.0);
            self.fps_accum = 0.0;
            self.fps_frames = 0;
        }
        self.ui_ctx.profiler.lock().push_ms(dt * 1000.0);

        // Fly cam when RMB held
        if self.rmb_down {
            let mut wish = Vec3::ZERO;
            if self.keys_wasd[0] {
                wish.z += 1.0;
            }
            if self.keys_wasd[2] {
                wish.z -= 1.0;
            }
            if self.keys_wasd[1] {
                wish.x -= 1.0;
            }
            if self.keys_wasd[3] {
                wish.x += 1.0;
            }
            if self.keys_wasd[5] {
                wish.y += 1.0;
            } // E up while flying
            if self.keys_wasd[4] {
                wish.y -= 1.0;
            } // Q down
            self.camera.lock().fly(wish, dt, self.shift_down);
        }

        // PIE: record + tick StateTree demo agent
        {
            let (record, tick_ai) = {
                let pie = self.ui_ctx.pie.lock();
                (pie.running, pie.is_running())
            };
            if record {
                let snap = self.registry.lock().snapshot_world(&self.world.lock());
                self.ui_ctx.time_travel.lock().push(snap);
            }
            if tick_ai && let Some(e) = *self.ui_ctx.demo_agent.lock() {
                let mut w = self.world.lock();
                if let Some(agent) = w.get_component_mut::<redox_gameplay::StateTreeComponent>(e) {
                    let dist = self.ui_ctx.graph.lock().distance_preview;
                    agent.tick_with_distance(dist, dt);
                    self.ui_ctx.graph.lock().sync_from_agent(agent);
                }
            }
        }

        // Auto world streaming (~4 Hz) while a session is open
        self.stream_accum += dt;
        if self.stream_accum >= 0.25 {
            self.stream_accum = 0.0;
            if let Some(session) = self.ui_ctx.world_session.lock().as_mut() {
                let eye = self.camera.lock().eye;
                session.tick(&mut self.world.lock(), eye);
            }
        }
    }

    fn drain_asset_spawn_requests(&mut self) {
        // Drain AssetBrowser spawn requests → Transform + mesh proxy marker
        for req in self.ui_ctx.drain_asset_spawns() {
            let label = req.label.clone().unwrap_or_else(|| {
                req.path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("asset")
                    .to_string()
            });
            let e = {
                let mut w = self.world.lock();
                let e = w.spawn();
                w.add_component(e, Transform::default());
                if req.kind == AssetKind::Mesh {
                    w.add_component(
                        e,
                        EditorMeshProxy {
                            path: req.path.clone(),
                            label: Some(label.clone()),
                        },
                    );
                }
                e
            };
            self.selection.lock().set_primary(Some(e));
            self.ui_ctx.log(format!(
                "[INFO] Spawned {} proxy for {}",
                label,
                req.path.display()
            ));
        }
    }

    fn handle_timetravel_status(&mut self) {
        {
            let status = self.ui_ctx.status.lock().clone();
            if let Some(rest) = status.strip_prefix("tt_scrub:") {
                if let Ok(idx) = rest.parse::<usize>() {
                    let mut world_g = self.world.lock();
                    let registry_g = self.registry.lock();
                    self.ui_ctx
                        .time_travel
                        .lock()
                        .restore(&mut world_g, &registry_g, idx);
                }
                self.ui_ctx.status.lock().clear();
            } else if status == "bake_report" {
                match self.ui_ctx.packager.lock().write_bake_report() {
                    Ok(msg) => self.ui_ctx.log(format!("[INFO] {msg}")),
                    Err(e) => self.ui_ctx.log(format!("[ERR] {e}")),
                }
                self.ui_ctx.status.lock().clear();
            }
        }
    }

    fn ensure_gpu(&mut self) {
        if self.shared_gpu.is_none() {
            let window_clone = self.window.clone();
            let boot = pollster::block_on(SharedGpu::request(window_clone));
            let host = RenderHost::new(boot.device, boot.queue, &boot.shared.adapter, 1280, 720);
            let mut renderer =
                egui_wgpu::Renderer::new(host.device(), boot.shared.config.format, None, 1);
            let lp = ViewportLinesPass::new(host.device());
            let tex_id = renderer.register_native_texture(
                host.device(),
                host.color_view(),
                wgpu::FilterMode::Linear,
            );
            self.egui_renderer = Some(renderer);
            self.lines_pass = Some(lp);
            self.viewport_tex_id = Some(tex_id);
            self.render_host = Some(host);
            self.shared_gpu = Some(boot.shared);
            self.ui_ctx.log(
                                "[INFO] SharedGpu + RenderHost 1280x720 (full Renderer PBR+VSM+bloom+Sharp AA; VG idle until VirtualMesh entities exist)",
                            );
        }
    }

    fn handle_world_status_requests(&mut self) {
        {
            let status = self.ui_ctx.status.lock().clone();
            if status == "git_refresh" {
                self.ui_ctx.git.lock().refresh(&self.world.lock());
                self.ui_ctx.log("[INFO] Git/OFPA status refreshed");
                self.ui_ctx.status.lock().clear();
            } else if let Some(path) = status.strip_prefix("open_world:") {
                // Agent E: World menu / suggest_world_roots → open_world:<path>
                match WorldSession::open_path(path) {
                    Ok(session) => {
                        self.ui_ctx.git.lock().set_world_root(&session.root);
                        self.ui_ctx.git.lock().refresh(&self.world.lock());
                        self.ui_ctx
                            .log(format!("[INFO] Opened world {}", session.root.display()));
                        *self.ui_ctx.world_session.lock() = Some(session);
                    }
                    Err(e) => {
                        self.ui_ctx.log(format!("[WARN] Open world failed: {e}"));
                    }
                }
                self.ui_ctx.status.lock().clear();
            } else if let Some(rest) = status.strip_prefix("snap:") {
                if let Ok(id) = rest.parse::<u32>()
                    && let Some(e) = resolve_entity(&self.world.lock(), id)
                {
                    let pos = self
                        .world
                        .lock()
                        .get_component::<Transform>(e)
                        .map(|t| t.translation)
                        .unwrap_or(Vec3::ZERO);
                    let (new_pos, new_rot) =
                        place_on_surface(pos, 0.5, Some(&mut self.physics.lock()), &[e]);
                    if let Some(t) = self.world.lock().get_component_mut::<Transform>(e) {
                        t.translation = new_pos;
                        t.rotation = new_rot;
                    }
                    self.ui_ctx.log(format!(
                        "[INFO] Surface snap entity #{id} → y={:.2}",
                        new_pos.y
                    ));
                    self.ui_ctx
                        .git
                        .lock()
                        .refresh_from_world(&self.world.lock());
                }
                self.ui_ctx.status.lock().clear();
            }
        }
    }

    fn handle_outliner_actions(&mut self) {
        {
            let actions: Vec<_> = std::mem::take(&mut *self.ui_ctx.pending_actions.lock());
            for action in actions {
                match action {
                    OutlinerAction::LoadCell(cell) => {
                        if let Some(session) = self.ui_ctx.world_session.lock().as_mut() {
                            match session.force_load(&mut self.world.lock(), cell) {
                                Ok(()) => self
                                    .ui_ctx
                                    .log(format!("[INFO] Loaded cell ({},{})", cell.x, cell.y)),
                                Err(e) => self.ui_ctx.log(format!("[WARN] Load cell failed: {e}")),
                            }
                        }
                    }
                    OutlinerAction::UnloadCell(cell) => {
                        if let Some(session) = self.ui_ctx.world_session.lock().as_mut() {
                            session.force_unload(&mut self.world.lock(), cell);
                            self.ui_ctx
                                .log(format!("[INFO] Unloaded cell ({},{})", cell.x, cell.y));
                        }
                    }
                    OutlinerAction::FocusActor { id, .. } => {
                        for e in self.world.lock().all_entities() {
                            if self
                                .world
                                .lock()
                                .get_component::<redox_scene::StreamedActor>(e)
                                .is_some_and(|a| a.id == id)
                            {
                                if let Some(t) =
                                    self.world.lock().get_component::<Transform>(e).copied()
                                {
                                    self.camera.lock().focus_on(
                                        redox_math::Aabb::from_center_size(
                                            t.translation,
                                            Vec3::splat(2.0),
                                        ),
                                    );
                                }
                                break;
                            }
                        }
                    }
                    OutlinerAction::RevertActor { cell, id } => {
                        let reverted = self.ui_ctx.git.lock().revert_actor(cell.x, cell.y, &id);
                        if let Some(session) = self.ui_ctx.world_session.lock().as_mut() {
                            session.force_unload(&mut self.world.lock(), cell);
                            let _ = session.force_load(&mut self.world.lock(), cell);
                        }
                        self.ui_ctx.log(format!(
                            "[INFO] Revert {id} cell ({},{}) git={}",
                            cell.x, cell.y, reverted
                        ));
                        self.ui_ctx
                            .git
                            .lock()
                            .refresh_from_world(&self.world.lock());
                    }
                }
            }
        }
    }

    /// Resize the RTT to last frame's dock viewport rect; re-register the egui texture.
    fn resize_viewport_target(&mut self) {
        let renderer = self.egui_renderer.as_mut().unwrap();
        if let (Some(host), Some(rect)) = (self.render_host.as_mut(), *self.viewport_rect.lock()) {
            let ppp = self.window.scale_factor() as f32;
            let w = (rect.width() * ppp).round().max(1.0) as u32;
            let h = (rect.height() * ppp).round().max(1.0) as u32;
            if host.size() != (w, h) {
                host.resize(w, h);
                self.camera.lock().set_viewport_size(w, h);
                if let Some(old) = self.viewport_tex_id.take() {
                    renderer.free_texture(&old);
                }
                self.viewport_tex_id = Some(renderer.register_native_texture(
                    host.device(),
                    host.color_view(),
                    wgpu::FilterMode::Linear,
                ));
            }
        }
    }

    fn render_viewport(
        &mut self,
        entities: &[(u32, Transform)],
        selected_id: Option<u32>,
        shading: ViewportShading,
        layout: ViewportLayout,
    ) {
        if let Some(host) = self.render_host.as_mut() {
            let cam = self.camera.lock().clone();
            match layout {
                ViewportLayout::Quad => {
                    host.render_quad(&cam, entities, selected_id, shading);
                }
                ViewportLayout::Single => {
                    host.render(&cam, entities, selected_id, shading);
                }
            }
            if let Some(lp) = self.lines_pass.as_ref()
                && layout == ViewportLayout::Single
            {
                let mut encoder =
                    host.device()
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("lines_encoder"),
                        });
                let gizmo_origin = selected_id.and_then(|id| {
                    entities
                        .iter()
                        .find(|(eid, _)| *eid == id)
                        .map(|(_, t)| t.translation)
                });
                let cell_size = self
                    .ui_ctx
                    .world_session
                    .lock()
                    .as_ref()
                    .map(|s| s.manifest.cell_size)
                    .unwrap_or(16.0);
                let mut overlay = build_editor_overlay_lines(gizmo_origin, cell_size);
                if *self.ui_ctx.show_git_ghosts.lock() {
                    let ghosts = self.ui_ctx.git.lock().ghosts.clone();
                    let w = self.world.lock();
                    for (id, old) in &ghosts {
                        overlay.extend(ghost_cube_lines(old.translation, [1.0, 0.85, 0.2, 0.7]));
                        for e in w.all_entities() {
                            if w.get_component::<redox_scene::StreamedActor>(e)
                                .is_some_and(|a| a.id == *id)
                            {
                                if let Some(t) = w.get_component::<Transform>(e) {
                                    overlay.push(ghost_move_line(old.translation, t.translation));
                                }
                                break;
                            }
                        }
                    }
                }
                lp.render(&mut encoder, &host.viewport, host.queue(), &cam, &overlay);
                host.queue().submit(Some(encoder.finish()));
            }
        }
    }

    fn pick_on_click(&mut self, entities: &[(u32, Transform)]) {
        let is_click = self.egui_ctx.input(|i| i.pointer.primary_clicked());
        let hover_pos = self.egui_ctx.input(|i| i.pointer.hover_pos());
        let alt = self.egui_ctx.input(|i| i.modifiers.alt);
        if is_click
            && !alt
            && self.gizmo.lock().axis.is_none()
            && let (Some(pos), Some(rect), Some(host)) = (
                hover_pos,
                *self.viewport_rect.lock(),
                self.render_host.as_ref(),
            )
        {
            let vp = &host.viewport;
            let ppp = self.window.scale_factor() as f32;
            if let Some((x, y)) = egui_to_viewport_coords(pos, rect, ppp, vp.size) {
                let brush_on = self.ui_ctx.brush.lock().active;
                let uv = redox_math::Vec2::new(
                    (x as f32 + 0.5) / vp.size.0 as f32,
                    (y as f32 + 0.5) / vp.size.1 as f32,
                );
                let cam = self.camera.lock().clone();
                let layout = *self.ui_ctx.layout.lock();
                let (origin, dir) = if layout == ViewportLayout::Quad {
                    let q = pick_in_quad_pane(uv, &cam, vp.size);
                    (q.origin, q.dir)
                } else {
                    redox_editor_core::gizmo::screen_ray(
                        uv,
                        cam.view_matrix(),
                        cam.proj_matrix(),
                        vp.size,
                    )
                };
                if brush_on {
                    // Hit y=0 plane
                    if dir.y.abs() > 1e-4 {
                        let t = -origin.y / dir.y;
                        if t > 0.0 {
                            let hit = origin + dir * t;
                            let (settings, kind) = {
                                let b = self.ui_ctx.brush.lock();
                                (b.settings.clone(), b.kind.clone())
                            };
                            let poses = scatter_poses(
                                hit,
                                settings.radius,
                                settings.density,
                                (hit.x * 1000.0) as u64 ^ (hit.z * 1000.0) as u64,
                            );
                            if let Some(session) = self.ui_ctx.world_session.lock().as_mut() {
                                let n = session.paint_actors(&mut self.world.lock(), &poses, &kind);
                                self.ui_ctx.log(format!(
                                    "[INFO] Brush painted {n} × {kind} at ({:.1},{:.1})",
                                    hit.x, hit.z
                                ));
                                self.ui_ctx
                                    .git
                                    .lock()
                                    .refresh_from_world(&self.world.lock());
                            } else {
                                let mut w = self.world.lock();
                                for (p, yaw) in poses {
                                    let e = w.spawn();
                                    w.add_component(
                                        e,
                                        Transform {
                                            translation: p + Vec3::Y * 0.5,
                                            rotation: redox_math::Quat::from_rotation_y(yaw),
                                            scale: Vec3::ONE,
                                        },
                                    );
                                }
                            }
                        }
                    }
                } else if let Some(id) = pick_unit_cubes(entities, origin, dir) {
                    if let Some(entity) = resolve_entity(&self.world.lock(), id) {
                        self.selection.lock().set_primary(Some(entity));
                        self.ui_ctx.log(format!(
                            "[INFO] Picked entity #{} at ({x},{y})",
                            entity.id()
                        ));
                    }
                } else {
                    self.selection.lock().clear();
                }
            }
        }
    }

    fn present(&mut self, full_output: egui::FullOutput) {
        let gpu = self.shared_gpu.as_ref().unwrap();
        let surf = &gpu.surface;
        let cfg = &gpu.config;
        let renderer = self.egui_renderer.as_mut().unwrap();
        self.egui_winit
            .handle_platform_output(&self.window, full_output.platform_output);

        let host = self.render_host.as_ref().unwrap();
        let dev = host.device();
        let que = host.queue();

        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [cfg.width, cfg.height],
            pixels_per_point: self.window.scale_factor() as f32,
        };
        for (id, image_delta) in &full_output.textures_delta.set {
            renderer.update_texture(dev, que, *id, image_delta);
        }

        let tris = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let mut encoder = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor_encoder"),
        });
        renderer.update_buffers(dev, que, &mut encoder, &tris, &screen_desc);
        let frame = match surf.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost) => {
                surf.configure(dev, cfg);
                return;
            }
            Err(e) => {
                log::warn!("surface error: {e:?}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("editor_egui_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.08,
                            g: 0.08,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            renderer.render(&mut rpass, &tris, &screen_desc);
        }
        for id in &full_output.textures_delta.free {
            renderer.free_texture(id);
        }
        que.submit(Some(encoder.finish()));
        frame.present();
        self.window.request_redraw();
    }
}
