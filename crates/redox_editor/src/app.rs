//! The editor application: owns all editor state and reacts to winit events.

use crate::cli::CliArgs;
use crate::commands::{focus_selection, run_command};
use crate::render_host::{RenderHost, SharedGpu};
use parking_lot::Mutex;
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_editor_core::{
    EditorCamera, GizmoMode, GizmoState, Selection, UndoStack, ViewportLinesPass, ViewportShading,
    WorldSession, egui_to_viewport_coords,
};
use redox_editor_ui::{EditorDock, EditorUiCtx};
use redox_math::{Transform, Vec3};
use redox_physics::PhysicsContext;
use redox_reflect::{TypeRegistry, register_engine_defaults};
use std::sync::Arc;
use std::time::Instant;

mod redraw;
mod ui;

/// All editor state that used to live in locals of `main`.
pub(crate) struct EditorApp {
    window: Arc<winit::window::Window>,
    dock: EditorDock,
    egui_ctx: egui::Context,
    egui_winit: egui_winit::State,
    selection: Arc<Mutex<Selection>>,
    world: Arc<Mutex<World>>,
    registry: Arc<Mutex<TypeRegistry>>,
    gizmo: Arc<Mutex<GizmoState>>,
    undo_stack: Arc<Mutex<UndoStack>>,
    camera: Arc<Mutex<EditorCamera>>,
    ui_ctx: Arc<EditorUiCtx>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    shared_gpu: Option<SharedGpu>,
    render_host: Option<RenderHost>,
    lines_pass: Option<ViewportLinesPass>,
    viewport_tex_id: Option<egui::TextureId>,
    viewport_rect: Arc<Mutex<Option<egui::Rect>>>,
    last_mouse_pos: Arc<Mutex<Option<egui::Pos2>>>,
    physics: Arc<Mutex<PhysicsContext>>,
    rmb_down: bool,
    alt_lmb_orbit: bool,
    keys_wasd: [bool; 6], // W A S D Q E
    shift_down: bool,
    last_frame: Instant,
    fps_accum: f32,
    fps_frames: u32,
    stream_accum: f32,
}

impl EditorApp {
    pub(crate) fn new(window: Arc<winit::window::Window>, cli: &CliArgs) -> Self {
        let dock = EditorDock::new();
        let egui_ctx = egui::Context::default();
        let egui_winit = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
        );

        let selection = Arc::new(Mutex::new(Selection::default()));
        let world = Arc::new(Mutex::new(World::new()));
        let registry = Arc::new(Mutex::new({
            let mut r = TypeRegistry::new();
            register_engine_defaults(&mut r);
            r.register::<redox_render::light::DirectionalLight>();
            r.register::<redox_render::light::PointLight>();
            r
        }));
        let gizmo = Arc::new(Mutex::new(GizmoState::new()));
        let undo_stack = Arc::new(Mutex::new(UndoStack::default()));
        let camera = Arc::new(Mutex::new(EditorCamera::default()));

        let ui_ctx = Arc::new(EditorUiCtx::new(
            world.clone(),
            selection.clone(),
            registry.clone(),
            gizmo.clone(),
            undo_stack.clone(),
            camera.clone(),
        ));

        // Seed: either open OFPA world or a few demo cubes
        let opened_world = WorldSession::open(&cli.world_root).ok();
        if let Some(mut session) = opened_world {
            // Load spawn neighborhood
            let spawn = redox_math::Vec3::new(56.0, 2.0, 56.0);
            camera.lock().eye = spawn + Vec3::new(12.0, 10.0, 12.0);
            camera.lock().pivot = spawn;
            {
                let mut w = world.lock();
                session.tick(&mut w, spawn);
                // Force a few cells around spawn for immediate editing
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let cell = redox_scene::PartitionCell {
                            x: 3 + dx,
                            y: 3 + dy,
                        };
                        let _ = session.force_load(&mut w, cell);
                    }
                }
            }
            ui_ctx.git.lock().set_world_root(&session.root);
            ui_ctx.git.lock().refresh(&world.lock());
            *ui_ctx.world_session.lock() = Some(session);
            ui_ctx.log(format!(
                "[INFO] Opened {} — OFPA cells loaded around spawn",
                cli.world_root
            ));
        } else {
            let mut w = world.lock();
            let positions = [
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(2.0, 0.5, 0.0),
                Vec3::new(-2.0, 0.5, 1.5),
                Vec3::new(0.0, 0.5, -3.0),
            ];
            let mut first = None;
            for p in positions {
                let e = w.spawn();
                w.add_component(e, Transform::from_translation(p));
                if first.is_none() {
                    first = Some(e);
                }
            }
            selection.lock().set_primary(first);
            ui_ctx.log(format!(
                "[INFO] No world at {} — demo cubes",
                cli.world_root
            ));
        }

        ui_ctx.spawn_demo_agent();

        let egui_renderer: Option<egui_wgpu::Renderer> = None;
        let shared_gpu: Option<SharedGpu> = None;
        let render_host: Option<RenderHost> = None;
        let lines_pass: Option<ViewportLinesPass> = None;
        let viewport_tex_id: Option<egui::TextureId> = None;
        let viewport_rect: Arc<Mutex<Option<egui::Rect>>> = Arc::new(Mutex::new(None));
        let last_mouse_pos: Arc<Mutex<Option<egui::Pos2>>> = Arc::new(Mutex::new(None));

        // Physics ground for surface snap (static infinite-ish box)
        let physics = Arc::new(Mutex::new({
            let mut phys = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
            let ground_e = {
                let mut w = world.lock();
                let e = w.spawn();
                w.add_component(e, Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)));
                e
            };
            let _ = phys.add_rigid_body(
                ground_e,
                RigidBodyBuilder::fixed().translation(redox_math::Vec3::new(0.0, -0.5, 0.0)),
                vec![ColliderBuilder::cuboid(64.0, 0.5, 64.0)],
            );
            phys
        }));

        let rmb_down = false;
        let alt_lmb_orbit = false;
        let keys_wasd = [false; 6]; // W A S D Q E
        let shift_down = false;
        let last_frame = Instant::now();
        let fps_accum = 0.0f32;
        let fps_frames = 0u32;
        let stream_accum = 0.0f32;

        Self {
            window,
            dock,
            egui_ctx,
            egui_winit,
            selection,
            world,
            registry,
            gizmo,
            undo_stack,
            camera,
            ui_ctx,
            egui_renderer,
            shared_gpu,
            render_host,
            lines_pass,
            viewport_tex_id,
            viewport_rect,
            last_mouse_pos,
            physics,
            rmb_down,
            alt_lmb_orbit,
            keys_wasd,
            shift_down,
            last_frame,
            fps_accum,
            fps_frames,
            stream_accum,
        }
    }

    pub(crate) fn handle_event(
        &mut self,
        event: winit::event::Event<()>,
        elwt: &winit::event_loop::EventLoopWindowTarget<()>,
    ) {
        match event {
            winit::event::Event::WindowEvent { event, .. } => {
                let _ = self.egui_winit.on_window_event(&self.window, &event);
                match event {
                    winit::event::WindowEvent::CloseRequested => {
                        let _ = self
                            .dock
                            .save_to_file(std::path::Path::new("editor_layout.ron"));
                        elwt.exit();
                    }
                    winit::event::WindowEvent::Resized(size) => self.on_resized(size),
                    winit::event::WindowEvent::MouseWheel { delta, .. } => {
                        self.on_mouse_wheel(delta)
                    }
                    winit::event::WindowEvent::CursorMoved { position, .. } => {
                        self.on_cursor_moved(position);
                    }
                    winit::event::WindowEvent::MouseInput { state, button, .. } => {
                        self.on_mouse_input(state, button);
                    }
                    winit::event::WindowEvent::KeyboardInput { event, .. } => {
                        self.on_keyboard(&event)
                    }
                    winit::event::WindowEvent::RedrawRequested => self.on_redraw(),
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => {
                self.window.request_redraw();
            }
            _ => {}
        }
    }

    fn on_resized(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if let (Some(gpu), Some(host)) = (self.shared_gpu.as_mut(), self.render_host.as_ref()) {
            gpu.resize(host.device(), size.width.max(1), size.height.max(1));
        }
    }

    fn on_mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        if self.rmb_down {
            let y = match delta {
                winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.01,
            };
            self.camera.lock().adjust_speed(y);
        }
    }

    fn on_cursor_moved(&mut self, position: winit::dpi::PhysicalPosition<f64>) {
        let pos = egui::Pos2::new(position.x as f32, position.y as f32);
        let prev = self.last_mouse_pos.lock().replace(pos);
        if let Some(prev) = prev {
            let dx = (pos.x - prev.x) * 0.005;
            let dy = (pos.y - prev.y) * 0.005;
            if self.rmb_down && !self.egui_ctx.wants_pointer_input() {
                self.camera.lock().look(dx, dy);
            } else if self.alt_lmb_orbit && !self.egui_ctx.wants_pointer_input() {
                self.camera.lock().orbit(dx, dy);
            }
        }
        // Gizmo drag
        if self.gizmo.lock().axis.is_some()
            && let (Some(sel), Some(host), Some(rect), Some(pos)) = (
                self.selection.lock().primary,
                self.render_host.as_ref(),
                *self.viewport_rect.lock(),
                *self.last_mouse_pos.lock(),
            )
        {
            let vp = &host.viewport;
            let ppp = self.window.scale_factor() as f32;
            if let Some((x, y)) = egui_to_viewport_coords(pos, rect, ppp, vp.size) {
                let uv =
                    redox_math::Vec2::new(x as f32 / vp.size.0 as f32, y as f32 / vp.size.1 as f32);
                let cam = self.camera.lock().clone();
                let (origin, dir) = redox_editor_core::gizmo::screen_ray(
                    uv,
                    cam.view_matrix(),
                    cam.proj_matrix(),
                    vp.size,
                );
                let entity_pos = self
                    .world
                    .lock()
                    .get_component::<Transform>(sel)
                    .map(|t| t.translation)
                    .unwrap_or(Vec3::ZERO);
                let cam_forward = cam.forward();
                if let Some(offset) =
                    self.gizmo
                        .lock()
                        .update_drag(origin, dir, entity_pos, cam_forward)
                {
                    let gizmo_guard = self.gizmo.lock();
                    let mut world_guard = self.world.lock();
                    let registry_guard = self.registry.lock();
                    let mut undo_guard = self.undo_stack.lock();
                    gizmo_guard.apply_drag(
                        &mut world_guard,
                        sel,
                        offset,
                        &registry_guard,
                        &mut undo_guard,
                    );
                }
            }
        }
    }

    fn on_mouse_input(
        &mut self,
        state: winit::event::ElementState,
        button: winit::event::MouseButton,
    ) {
        let pressed = state == winit::event::ElementState::Pressed;
        match button {
            winit::event::MouseButton::Right => self.rmb_down = pressed,
            winit::event::MouseButton::Left => {
                let alt = self.egui_ctx.input(|i| i.modifiers.alt);
                if pressed && alt {
                    self.alt_lmb_orbit = true;
                } else if !pressed {
                    self.alt_lmb_orbit = false;
                }
                if !self.egui_ctx.wants_pointer_input() && pressed && !alt {
                    if let (Some(sel), Some(host), Some(rect), Some(pos)) = (
                        self.selection.lock().primary,
                        self.render_host.as_ref(),
                        *self.viewport_rect.lock(),
                        *self.last_mouse_pos.lock(),
                    ) {
                        let vp = &host.viewport;
                        let ppp = self.window.scale_factor() as f32;
                        if let Some((x, y)) = egui_to_viewport_coords(pos, rect, ppp, vp.size) {
                            let uv = redox_math::Vec2::new(
                                x as f32 / vp.size.0 as f32,
                                y as f32 / vp.size.1 as f32,
                            );
                            let cam = self.camera.lock().clone();
                            let (origin, dir) = redox_editor_core::gizmo::screen_ray(
                                uv,
                                cam.view_matrix(),
                                cam.proj_matrix(),
                                vp.size,
                            );
                            if let Some(t) =
                                self.world.lock().get_component::<Transform>(sel).copied()
                            {
                                let forward = cam.forward();
                                let mut gizmo_guard = self.gizmo.lock();
                                if gizmo_guard.start_drag(origin, dir, t.translation, t, forward) {
                                    self.ui_ctx
                                        .log(format!("[INFO] Gizmo drag {:?}", gizmo_guard.axis));
                                }
                            }
                        }
                    }
                } else if !pressed && self.gizmo.lock().axis.is_some() {
                    self.gizmo.lock().end_drag();
                    if *self.ui_ctx.surface_snap.lock()
                        && let Some(e) = self.selection.lock().primary
                    {
                        *self.ui_ctx.status.lock() = format!("snap:{}", e.id());
                    }
                    self.ui_ctx
                        .git
                        .lock()
                        .refresh_from_world(&self.world.lock());
                }
            }
            _ => {}
        }
    }

    fn on_keyboard(&mut self, event: &winit::event::KeyEvent) {
        let pressed = event.state == winit::event::ElementState::Pressed;
        match event.physical_key {
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyW) => {
                self.keys_wasd[0] = pressed;
                if pressed && !self.rmb_down && !self.egui_ctx.wants_keyboard_input() {
                    self.gizmo.lock().mode = GizmoMode::Translate;
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyA) => {
                self.keys_wasd[1] = pressed;
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyS) => {
                self.keys_wasd[2] = pressed;
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyD) => {
                self.keys_wasd[3] = pressed;
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyQ) => {
                self.keys_wasd[4] = pressed;
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyE) => {
                self.keys_wasd[5] = pressed;
                if pressed && !self.rmb_down && !self.egui_ctx.wants_keyboard_input() {
                    self.gizmo.lock().mode = GizmoMode::Rotate;
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyR) => {
                if pressed && !self.rmb_down && !self.egui_ctx.wants_keyboard_input() {
                    self.gizmo.lock().mode = GizmoMode::Scale;
                }
            }
            winit::keyboard::PhysicalKey::Code(
                winit::keyboard::KeyCode::ShiftLeft | winit::keyboard::KeyCode::ShiftRight,
            ) => {
                self.shift_down = pressed;
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Delete) => {
                if pressed && !self.egui_ctx.wants_keyboard_input() {
                    run_command(&self.ui_ctx, &mut self.dock, "delete");
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyF) => {
                if pressed && !self.egui_ctx.wants_keyboard_input() {
                    focus_selection(&self.ui_ctx);
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::End) => {
                if pressed && !self.egui_ctx.wants_keyboard_input() {
                    run_command(&self.ui_ctx, &mut self.dock, "align_surface");
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyB) => {
                if pressed && !self.egui_ctx.wants_keyboard_input() {
                    let mut b = self.ui_ctx.brush.lock();
                    b.active = !b.active;
                    self.ui_ctx.log(format!(
                        "[INFO] Brush {}",
                        if b.active { "ON" } else { "OFF" }
                    ));
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F1) => {
                if pressed {
                    *self.ui_ctx.shading.lock() = ViewportShading::Lit;
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F2) => {
                if pressed {
                    *self.ui_ctx.shading.lock() = ViewportShading::Unlit;
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F3) => {
                if pressed {
                    *self.ui_ctx.shading.lock() = ViewportShading::Normals;
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F4) => {
                if pressed {
                    *self.ui_ctx.shading.lock() = ViewportShading::Wireframe;
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F5) => {
                if pressed {
                    let mut l = self.ui_ctx.layout.lock();
                    *l = l.toggle();
                    self.ui_ctx.log(format!("[INFO] Layout {:?}", *l));
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F6) => {
                if pressed {
                    let mut d = self.ui_ctx.debug_flags.lock();
                    d.apply_f_key(6);
                    self.ui_ctx.log(format!(
                        "[INFO] Overdraw {}",
                        if d.show_overdraw { "ON" } else { "OFF" }
                    ));
                }
            }
            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F7) => {
                if pressed {
                    let mut d = self.ui_ctx.debug_flags.lock();
                    d.apply_f_key(7);
                    self.ui_ctx.log(format!(
                        "[INFO] AABBs {}",
                        if d.show_aabbs { "ON" } else { "OFF" }
                    ));
                }
            }
            _ => {}
        }
    }
}
