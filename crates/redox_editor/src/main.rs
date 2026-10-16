use parking_lot::Mutex;
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_editor_core::{
    EditorCamera, GizmoMode, GizmoState, Selection, UndoStack, ViewportLayout, ViewportLinesPass,
    ViewportShading, WorldSession, build_editor_overlay_lines, egui_to_viewport_coords,
    ghost_cube_lines, ghost_move_line, pick_in_quad_pane, pick_unit_cubes, place_on_surface,
    scatter_poses,
};
use redox_editor_ui::{AssetKind, EditorDock, EditorMeshProxy, EditorUiCtx, OutlinerAction};
use redox_math::{Transform, Vec3};
use redox_physics::PhysicsContext;
use redox_reflect::{TypeRegistry, register_engine_defaults};
use std::sync::Arc;
use std::time::Instant;

mod render_host;
use render_host::{RenderHost, SharedGpu};

fn resolve_entity(world: &World, id: u32) -> Option<Entity> {
    world.all_entities().find(|e| e.id() == id)
}

fn collect_transforms(world: &World) -> Vec<(u32, Transform)> {
    let mut out = Vec::new();
    for e in world.all_entities() {
        if let Some(t) = world.get_component::<Transform>(e).copied() {
            // Hide physics ground slab (y < 0)
            if t.translation.y < -0.05 {
                continue;
            }
            out.push((e.id(), t));
        }
    }
    out
}

fn focus_selection(ui_ctx: &EditorUiCtx) {
    let sel = ui_ctx.selection.lock().primary;
    if let Some(e) = sel
        && let Some(t) = ui_ctx.world.lock().get_component::<Transform>(e).copied()
    {
        ui_ctx
            .camera
            .lock()
            .focus_on(redox_math::Aabb::from_center_size(
                t.translation,
                Vec3::splat(1.0),
            ));
    }
}

fn run_command(ui_ctx: &EditorUiCtx, dock: &mut EditorDock, id: &str) {
    match id {
        "focus" => focus_selection(ui_ctx),
        "create_entity" => {
            let e = {
                let mut w = ui_ctx.world.lock();
                let e = w.spawn();
                w.add_component(e, Transform::default());
                e
            };
            ui_ctx.selection.lock().set_primary(Some(e));
            ui_ctx.log(format!("[INFO] Created entity #{}", e.id()));
        }
        "duplicate" => {
            let sel = ui_ctx.selection.lock().primary;
            if let Some(src) = sel {
                let t = ui_ctx
                    .world
                    .lock()
                    .get_component::<Transform>(src)
                    .copied()
                    .unwrap_or_default();
                let mut nt = t;
                nt.translation.x += 1.0;
                let e = {
                    let mut w = ui_ctx.world.lock();
                    let e = w.spawn();
                    w.add_component(e, nt);
                    e
                };
                ui_ctx.selection.lock().set_primary(Some(e));
                ui_ctx.log(format!("[INFO] Duplicated #{} → #{}", src.id(), e.id()));
            }
        }
        "delete" => {
            let sel = ui_ctx.selection.lock().primary.take();
            if let Some(e) = sel {
                ui_ctx.world.lock().despawn(e);
                ui_ctx.selection.lock().clear();
                ui_ctx.log(format!("[INFO] Deleted entity #{}", e.id()));
            }
        }
        "reset_layout" => {
            dock.reset_layout();
            ui_ctx.log("[INFO] Layout reset");
        }
        "pie_play" => {
            let mut pie = ui_ctx.pie.lock();
            let world = ui_ctx.world.lock();
            let registry = ui_ctx.registry.lock();
            if pie.running && pie.paused {
                pie.resume();
            } else if !pie.running {
                pie.start(&world, &registry);
                ui_ctx.log("[INFO] PIE started");
            }
        }
        "pie_stop" => {
            let mut pie = ui_ctx.pie.lock();
            let mut world = ui_ctx.world.lock();
            let registry = ui_ctx.registry.lock();
            pie.stop(&mut world, &registry);
            ui_ctx.time_travel.lock().frames.clear();
            ui_ctx.log("[INFO] PIE stopped — world restored");
        }
        "bake_meshlets" => {
            ui_ctx.log("[WARN] Bake Meshlets not wired yet — use Packager for OFPA report")
        }
        "bake_report" => *ui_ctx.status.lock() = "bake_report".into(),
        "spawn_agent" => ui_ctx.spawn_demo_agent(),
        "shading_lit" => {
            *ui_ctx.shading.lock() = ViewportShading::Lit;
            ui_ctx.log("[INFO] Shading: Lit");
        }
        "shading_unlit" => {
            *ui_ctx.shading.lock() = ViewportShading::Unlit;
            ui_ctx.log("[INFO] Shading: Unlit");
        }
        "shading_normals" => {
            *ui_ctx.shading.lock() = ViewportShading::Normals;
            ui_ctx.log("[INFO] Shading: Normals");
        }
        "shading_wire" => {
            *ui_ctx.shading.lock() = ViewportShading::Wireframe;
            ui_ctx.log("[INFO] Shading: Wireframe");
        }
        "align_surface" => {
            if let Some(e) = ui_ctx.selection.lock().primary {
                *ui_ctx.status.lock() = format!("snap:{}", e.id());
            }
        }
        other => ui_ctx.log(format!("[WARN] Unknown command: {other}")),
    }
}

const DEFAULT_WORLD_ROOT: &str = "worlds/arena";

const HELP_TEXT: &str = "RedOx Studio Editor\n  --help            Show this help\n  --bake            Bake meshlets + SDF and write a report (headless, then exit)\n  --world <path>    World root (default worlds/arena)";

/// Parsed command line. `--world <path>` selects the world root.
struct CliArgs {
    help: bool,
    bake: bool,
    world_root: String,
}

fn parse_cli(args: &[String]) -> Result<CliArgs, String> {
    let mut cli = CliArgs {
        help: false,
        bake: false,
        world_root: DEFAULT_WORLD_ROOT.to_string(),
    };
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => cli.help = true,
            "--bake" => cli.bake = true,
            "--world" => {
                cli.world_root = iter
                    .next()
                    .ok_or_else(|| "--world requires a path".to_string())?
                    .clone();
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(cli)
}

/// Headless bake: meshlet cache, SDF field and the OFPA scan report. No window or GPU.
fn run_bake(world_root: &str) -> Result<(), String> {
    let mut packager = redox_editor_ui::packager::PackagerPanel::new(world_root);
    let steps: [(&str, Result<String, anyhow::Error>); 3] = [
        ("meshlets", packager.bake_meshlets()),
        ("sdf", packager.bake_sdf()),
        ("report", packager.write_bake_report()),
    ];
    let mut failed = false;
    for (name, result) in steps {
        match result {
            Ok(msg) => println!("[bake:{name}] {msg}"),
            Err(e) => {
                eprintln!("[bake:{name}] FAILED: {e:#}");
                failed = true;
            }
        }
    }
    if failed {
        Err("one or more bake steps failed".to_string())
    } else {
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cli = match parse_cli(&args) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{e}\n{HELP_TEXT}");
            std::process::exit(2);
        }
    };
    if cli.help {
        println!("{HELP_TEXT}");
        return;
    }
    if cli.bake {
        if let Err(e) = run_bake(&cli.world_root) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    env_logger::init();
    log::info!("RedOx Studio Editor starting...");

    let event_loop = winit::event_loop::EventLoop::new().unwrap();
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("RedOx Studio — Editor")
            .with_inner_size(winit::dpi::LogicalSize::new(1600, 900))
            .build(&event_loop)
            .unwrap(),
    );

    let mut dock = EditorDock::new();
    let egui_ctx = egui::Context::default();
    let mut egui_winit = egui_winit::State::new(
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

    let mut egui_renderer: Option<egui_wgpu::Renderer> = None;
    let mut shared_gpu: Option<SharedGpu> = None;
    let mut render_host: Option<RenderHost> = None;
    let mut lines_pass: Option<ViewportLinesPass> = None;
    let mut viewport_tex_id: Option<egui::TextureId> = None;
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
            RigidBodyBuilder::fixed().translation(rapier3d::na::Vector3::new(0.0, -0.5, 0.0)),
            vec![ColliderBuilder::cuboid(64.0, 0.5, 64.0)],
        );
        phys
    }));

    let mut rmb_down = false;
    let mut alt_lmb_orbit = false;
    let mut keys_wasd = [false; 6]; // W A S D Q E
    let mut shift_down = false;
    let mut last_frame = Instant::now();
    let mut fps_accum = 0.0f32;
    let mut fps_frames = 0u32;
    let mut stream_accum = 0.0f32;

    let _ = event_loop.run(move |event, elwt| {
        match event {
            winit::event::Event::WindowEvent { event, .. } => {
                let response = egui_winit.on_window_event(&window, &event);
                match event {
                    winit::event::WindowEvent::CloseRequested => {
                        let _ = dock.save_to_file(std::path::Path::new("editor_layout.ron"));
                        elwt.exit();
                    }
                    winit::event::WindowEvent::Resized(size) => {
                        if let (Some(gpu), Some(host)) =
                            (shared_gpu.as_mut(), render_host.as_ref())
                        {
                            gpu.resize(host.device(), size.width.max(1), size.height.max(1));
                        }
                    }
                    winit::event::WindowEvent::MouseWheel { delta, .. } => {
                        if rmb_down {
                            let y = match delta {
                                winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                                winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.01,
                            };
                            camera.lock().adjust_speed(y);
                        }
                    }
                    winit::event::WindowEvent::CursorMoved { position, .. } => {
                        let pos = egui::Pos2::new(position.x as f32, position.y as f32);
                        let prev = last_mouse_pos.lock().replace(pos);
                        if let Some(prev) = prev {
                            let dx = (pos.x - prev.x) * 0.005;
                            let dy = (pos.y - prev.y) * 0.005;
                            if rmb_down && !egui_ctx.wants_pointer_input() {
                                camera.lock().look(dx, dy);
                            } else if alt_lmb_orbit && !egui_ctx.wants_pointer_input() {
                                camera.lock().orbit(dx, dy);
                            }
                        }
                        // Gizmo drag
                        if gizmo.lock().axis.is_some()
                            && let (Some(sel), Some(host), Some(rect), Some(pos)) = (
                                selection.lock().primary,
                                render_host.as_ref(),
                                *viewport_rect.lock(),
                                *last_mouse_pos.lock(),
                            ) {
                                let vp = &host.viewport;
                                let ppp = window.scale_factor() as f32;
                                if let Some((x, y)) = egui_to_viewport_coords(pos, rect, ppp, vp.size) {
                                    let uv = redox_math::Vec2::new(
                                        x as f32 / vp.size.0 as f32,
                                        y as f32 / vp.size.1 as f32,
                                    );
                                    let cam = camera.lock().clone();
                                    let (origin, dir) = redox_editor_core::gizmo::screen_ray(
                                        uv,
                                        cam.view_matrix(),
                                        cam.proj_matrix(),
                                        vp.size,
                                    );
                                    let entity_pos = world
                                        .lock()
                                        .get_component::<Transform>(sel)
                                        .map(|t| t.translation)
                                        .unwrap_or(Vec3::ZERO);
                                    let cam_forward = cam.forward();
                                    if let Some(offset) =
                                        gizmo.lock().update_drag(origin, dir, entity_pos, cam_forward)
                                    {
                                        let gizmo_guard = gizmo.lock();
                                        let mut world_guard = world.lock();
                                        let registry_guard = registry.lock();
                                        let mut undo_guard = undo_stack.lock();
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
                    winit::event::WindowEvent::MouseInput { state, button, .. } => {
                        let pressed = state == winit::event::ElementState::Pressed;
                        match button {
                            winit::event::MouseButton::Right => rmb_down = pressed,
                            winit::event::MouseButton::Left => {
                                let alt = egui_ctx.input(|i| i.modifiers.alt);
                                if pressed && alt {
                                    alt_lmb_orbit = true;
                                } else if !pressed {
                                    alt_lmb_orbit = false;
                                }
                                if !egui_ctx.wants_pointer_input() && pressed && !alt {
                                    if let (Some(sel), Some(host), Some(rect), Some(pos)) = (
                                        selection.lock().primary,
                                        render_host.as_ref(),
                                        *viewport_rect.lock(),
                                        *last_mouse_pos.lock(),
                                    ) {
                                        let vp = &host.viewport;
                                        let ppp = window.scale_factor() as f32;
                                        if let Some((x, y)) =
                                            egui_to_viewport_coords(pos, rect, ppp, vp.size)
                                        {
                                            let uv = redox_math::Vec2::new(
                                                x as f32 / vp.size.0 as f32,
                                                y as f32 / vp.size.1 as f32,
                                            );
                                            let cam = camera.lock().clone();
                                            let (origin, dir) = redox_editor_core::gizmo::screen_ray(
                                                uv,
                                                cam.view_matrix(),
                                                cam.proj_matrix(),
                                                vp.size,
                                            );
                                            if let Some(t) =
                                                world.lock().get_component::<Transform>(sel).copied()
                                            {
                                                let forward = cam.forward();
                                                let mut gizmo_guard = gizmo.lock();
                                                if gizmo_guard.start_drag(
                                                    origin,
                                                    dir,
                                                    t.translation,
                                                    t,
                                                    forward,
                                                ) {
                                                    ui_ctx.log(format!(
                                                        "[INFO] Gizmo drag {:?}",
                                                        gizmo_guard.axis
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                } else if !pressed && gizmo.lock().axis.is_some() {
                                    gizmo.lock().end_drag();
                                    if *ui_ctx.surface_snap.lock()
                                        && let Some(e) = selection.lock().primary {
                                            *ui_ctx.status.lock() = format!("snap:{}", e.id());
                                        }
                                    ui_ctx.git.lock().refresh_from_world(&world.lock());
                                }
                            }
                            _ => {}
                        }
                    }
                    winit::event::WindowEvent::KeyboardInput { event, .. } => {
                        let pressed = event.state == winit::event::ElementState::Pressed;
                        match event.physical_key {
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyW) => {
                                keys_wasd[0] = pressed;
                                if pressed && !rmb_down && !egui_ctx.wants_keyboard_input() {
                                    gizmo.lock().mode = GizmoMode::Translate;
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyA) => {
                                keys_wasd[1] = pressed;
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyS) => {
                                keys_wasd[2] = pressed;
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyD) => {
                                keys_wasd[3] = pressed;
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyQ) => {
                                keys_wasd[4] = pressed;
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyE) => {
                                keys_wasd[5] = pressed;
                                if pressed && !rmb_down && !egui_ctx.wants_keyboard_input() {
                                    gizmo.lock().mode = GizmoMode::Rotate;
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyR) => {
                                if pressed && !rmb_down && !egui_ctx.wants_keyboard_input() {
                                    gizmo.lock().mode = GizmoMode::Scale;
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(
                                winit::keyboard::KeyCode::ShiftLeft
                                | winit::keyboard::KeyCode::ShiftRight,
                            ) => {
                                shift_down = pressed;
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Delete) => {
                                if pressed && !egui_ctx.wants_keyboard_input() {
                                    run_command(&ui_ctx, &mut dock, "delete");
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyF) => {
                                if pressed && !egui_ctx.wants_keyboard_input() {
                                    focus_selection(&ui_ctx);
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::End) => {
                                if pressed && !egui_ctx.wants_keyboard_input() {
                                    run_command(&ui_ctx, &mut dock, "align_surface");
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyB) => {
                                if pressed && !egui_ctx.wants_keyboard_input() {
                                    let mut b = ui_ctx.brush.lock();
                                    b.active = !b.active;
                                    ui_ctx.log(format!(
                                        "[INFO] Brush {}",
                                        if b.active { "ON" } else { "OFF" }
                                    ));
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F1) => {
                                if pressed {
                                    *ui_ctx.shading.lock() = ViewportShading::Lit;
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F2) => {
                                if pressed {
                                    *ui_ctx.shading.lock() = ViewportShading::Unlit;
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F3) => {
                                if pressed {
                                    *ui_ctx.shading.lock() = ViewportShading::Normals;
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F4) => {
                                if pressed {
                                    *ui_ctx.shading.lock() = ViewportShading::Wireframe;
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F5) => {
                                if pressed {
                                    let mut l = ui_ctx.layout.lock();
                                    *l = l.toggle();
                                    ui_ctx.log(format!("[INFO] Layout {:?}", *l));
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F6) => {
                                if pressed {
                                    let mut d = ui_ctx.debug_flags.lock();
                                    d.apply_f_key(6);
                                    ui_ctx.log(format!(
                                        "[INFO] Overdraw {}",
                                        if d.show_overdraw { "ON" } else { "OFF" }
                                    ));
                                }
                            }
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F7) => {
                                if pressed {
                                    let mut d = ui_ctx.debug_flags.lock();
                                    d.apply_f_key(7);
                                    ui_ctx.log(format!(
                                        "[INFO] AABBs {}",
                                        if d.show_aabbs { "ON" } else { "OFF" }
                                    ));
                                }
                            }
                            _ => {}
                        }
                        let _ = response;
                    }
                    winit::event::WindowEvent::RedrawRequested => {
                        let dt = last_frame.elapsed().as_secs_f32().max(1e-4);
                        last_frame = Instant::now();
                        fps_accum += dt;
                        fps_frames += 1;
                        if fps_accum >= 0.25 {
                            let fps = fps_frames as f32 / fps_accum;
                            *ui_ctx.fps.lock() = fps;
                            *ui_ctx.frame_ms.lock() = 1000.0 / fps.max(1.0);
                            fps_accum = 0.0;
                            fps_frames = 0;
                        }
                        ui_ctx.profiler.lock().push_ms(dt * 1000.0);

                        // Fly cam when RMB held
                        if rmb_down {
                            let mut wish = Vec3::ZERO;
                            if keys_wasd[0] {
                                wish.z += 1.0;
                            }
                            if keys_wasd[2] {
                                wish.z -= 1.0;
                            }
                            if keys_wasd[1] {
                                wish.x -= 1.0;
                            }
                            if keys_wasd[3] {
                                wish.x += 1.0;
                            }
                            if keys_wasd[5] {
                                wish.y += 1.0;
                            } // E up while flying
                            if keys_wasd[4] {
                                wish.y -= 1.0;
                            } // Q down
                            camera.lock().fly(wish, dt, shift_down);
                        }

                        // PIE: record + tick StateTree demo agent
                        {
                            let (record, tick_ai) = {
                                let pie = ui_ctx.pie.lock();
                                (pie.running, pie.is_running())
                            };
                            if record {
                                let snap = registry.lock().snapshot_world(&world.lock());
                                ui_ctx.time_travel.lock().push(snap);
                            }
                            if tick_ai
                                && let Some(e) = *ui_ctx.demo_agent.lock() {
                                    let mut w = world.lock();
                                    if let Some(agent) = w
                                        .get_component_mut::<redox_gameplay::StateTreeComponent>(e)
                                    {
                                        let dist = ui_ctx.graph.lock().distance_preview;
                                        agent.tick_with_distance(dist, dt);
                                        ui_ctx.graph.lock().sync_from_agent(agent);
                                    }
                                }
                        }

                        // Auto world streaming (~4 Hz) while a session is open
                        stream_accum += dt;
                        if stream_accum >= 0.25 {
                            stream_accum = 0.0;
                            if let Some(session) = ui_ctx.world_session.lock().as_mut() {
                                let eye = camera.lock().eye;
                                session.tick(&mut world.lock(), eye);
                            }
                        }

                        // Drain AssetBrowser spawn requests → Transform + mesh proxy marker
                        for req in ui_ctx.drain_asset_spawns() {
                            let label = req
                                .label
                                .clone()
                                .unwrap_or_else(|| {
                                    req.path
                                        .file_stem()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or("asset")
                                        .to_string()
                                });
                            let e = {
                                let mut w = world.lock();
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
                            selection.lock().set_primary(Some(e));
                            ui_ctx.log(format!(
                                "[INFO] Spawned {} proxy for {}",
                                label,
                                req.path.display()
                            ));
                        }

                        // Time-travel scrub / bake report status
                        {
                            let status = ui_ctx.status.lock().clone();
                            if let Some(rest) = status.strip_prefix("tt_scrub:") {
                                if let Ok(idx) = rest.parse::<usize>() {
                                    let mut world_g = world.lock();
                                    let registry_g = registry.lock();
                                    ui_ctx.time_travel.lock().restore(&mut world_g, &registry_g, idx);
                                }
                                ui_ctx.status.lock().clear();
                            } else if status == "bake_report" {
                                match ui_ctx.packager.lock().write_bake_report() {
                                    Ok(msg) => ui_ctx.log(format!("[INFO] {msg}")),
                                    Err(e) => ui_ctx.log(format!("[ERR] {e}")),
                                }
                                ui_ctx.status.lock().clear();
                            }
                        }

                        if shared_gpu.is_none() {
                            let window_clone = window.clone();
                            let boot = pollster::block_on(SharedGpu::request(window_clone));
                            let host = RenderHost::new(
                                boot.device,
                                boot.queue,
                                &boot.shared.adapter,
                                1280,
                                720,
                            );
                            let mut renderer = egui_wgpu::Renderer::new(
                                host.device(),
                                boot.shared.config.format,
                                None,
                                1,
                            );
                            let lp = ViewportLinesPass::new(host.device());
                            let tex_id = renderer.register_native_texture(
                                host.device(),
                                host.color_view(),
                                wgpu::FilterMode::Linear,
                            );
                            egui_renderer = Some(renderer);
                            lines_pass = Some(lp);
                            viewport_tex_id = Some(tex_id);
                            render_host = Some(host);
                            shared_gpu = Some(boot.shared);
                            ui_ctx.log(
                                "[INFO] SharedGpu + RenderHost 1280x720 (full Renderer PBR+VSM+bloom+Sharp AA; VG idle until VirtualMesh entities exist)",
                            );
                        }

                        let gpu = shared_gpu.as_ref().unwrap();
                        let surf = &gpu.surface;
                        let cfg = &gpu.config;
                        let renderer = egui_renderer.as_mut().unwrap();

                        // Surface snap requests (End / Align command)
                        {
                            let status = ui_ctx.status.lock().clone();
                            if status == "git_refresh" {
                                ui_ctx.git.lock().refresh(&world.lock());
                                ui_ctx.log("[INFO] Git/OFPA status refreshed");
                                ui_ctx.status.lock().clear();
                            } else if let Some(path) = status.strip_prefix("open_world:") {
                                // Agent E: World menu / suggest_world_roots → open_world:<path>
                                match WorldSession::open_path(path) {
                                    Ok(session) => {
                                        ui_ctx.git.lock().set_world_root(&session.root);
                                        ui_ctx.git.lock().refresh(&world.lock());
                                        ui_ctx.log(format!(
                                            "[INFO] Opened world {}",
                                            session.root.display()
                                        ));
                                        *ui_ctx.world_session.lock() = Some(session);
                                    }
                                    Err(e) => {
                                        ui_ctx.log(format!("[WARN] Open world failed: {e}"));
                                    }
                                }
                                ui_ctx.status.lock().clear();
                            } else if let Some(rest) = status.strip_prefix("snap:") {
                                if let Ok(id) = rest.parse::<u32>()
                                    && let Some(e) = resolve_entity(&world.lock(), id) {
                                        let pos = world
                                            .lock()
                                            .get_component::<Transform>(e)
                                            .map(|t| t.translation)
                                            .unwrap_or(Vec3::ZERO);
                                        let (new_pos, new_rot) = place_on_surface(
                                            pos,
                                            0.5,
                                            Some(&mut physics.lock()),
                                            &[e],
                                        );
                                        if let Some(t) =
                                            world.lock().get_component_mut::<Transform>(e)
                                        {
                                            t.translation = new_pos;
                                            t.rotation = new_rot;
                                        }
                                        ui_ctx.log(format!(
                                            "[INFO] Surface snap entity #{id} → y={:.2}",
                                            new_pos.y
                                        ));
                                        ui_ctx.git.lock().refresh_from_world(&world.lock());
                                    }
                                ui_ctx.status.lock().clear();
                            }
                        }

                        // Outliner Load/Unload/Revert
                        {
                            let actions: Vec<_> =
                                std::mem::take(&mut *ui_ctx.pending_actions.lock());
                            for action in actions {
                                match action {
                                    OutlinerAction::LoadCell(cell) => {
                                        if let Some(session) =
                                            ui_ctx.world_session.lock().as_mut()
                                        {
                                            match session.force_load(&mut world.lock(), cell) {
                                                Ok(()) => ui_ctx.log(format!(
                                                    "[INFO] Loaded cell ({},{})",
                                                    cell.x, cell.y
                                                )),
                                                Err(e) => ui_ctx
                                                    .log(format!("[WARN] Load cell failed: {e}")),
                                            }
                                        }
                                    }
                                    OutlinerAction::UnloadCell(cell) => {
                                        if let Some(session) =
                                            ui_ctx.world_session.lock().as_mut()
                                        {
                                            session.force_unload(&mut world.lock(), cell);
                                            ui_ctx.log(format!(
                                                "[INFO] Unloaded cell ({},{})",
                                                cell.x, cell.y
                                            ));
                                        }
                                    }
                                    OutlinerAction::FocusActor { id, .. } => {
                                        for e in world.lock().all_entities() {
                                            if world
                                                .lock()
                                                .get_component::<redox_scene::StreamedActor>(e)
                                                .is_some_and(|a| a.id == id)
                                            {
                                                if let Some(t) = world
                                                    .lock()
                                                    .get_component::<Transform>(e)
                                                    .copied()
                                                {
                                                    camera.lock().focus_on(
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
                                        let reverted = ui_ctx.git.lock().revert_actor(
                                            cell.x,
                                            cell.y,
                                            &id,
                                        );
                                        if let Some(session) =
                                            ui_ctx.world_session.lock().as_mut()
                                        {
                                            session.force_unload(&mut world.lock(), cell);
                                            let _ = session.force_load(&mut world.lock(), cell);
                                        }
                                        ui_ctx.log(format!(
                                            "[INFO] Revert {id} cell ({},{}) git={}",
                                            cell.x, cell.y, reverted
                                        ));
                                        ui_ctx.git.lock().refresh_from_world(&world.lock());
                                    }
                                }
                            }
                        }

                        let entities = collect_transforms(&world.lock());
                        let selected_id = selection.lock().primary.map(|e| e.id());
                        let shading = *ui_ctx.shading.lock();
                        let layout = *ui_ctx.layout.lock();

                        // Resize RTT to last frame's dock viewport rect; re-register egui texture.
                        if let (Some(host), Some(rect)) =
                            (render_host.as_mut(), *viewport_rect.lock())
                        {
                            let ppp = window.scale_factor() as f32;
                            let w = (rect.width() * ppp).round().max(1.0) as u32;
                            let h = (rect.height() * ppp).round().max(1.0) as u32;
                            if host.size() != (w, h) {
                                host.resize(w, h);
                                camera.lock().set_viewport_size(w, h);
                                if let Some(old) = viewport_tex_id.take() {
                                    renderer.free_texture(&old);
                                }
                                viewport_tex_id = Some(renderer.register_native_texture(
                                    host.device(),
                                    host.color_view(),
                                    wgpu::FilterMode::Linear,
                                ));
                            }
                        }

                        if let Some(host) = render_host.as_mut() {
                            let cam = camera.lock().clone();
                            match layout {
                                ViewportLayout::Quad => {
                                    host.render_quad(&cam, &entities, selected_id, shading);
                                }
                                ViewportLayout::Single => {
                                    host.render(&cam, &entities, selected_id, shading);
                                }
                            }
                            if let Some(lp) = lines_pass.as_ref()
                                && layout == ViewportLayout::Single {
                                    let mut encoder = host.device().create_command_encoder(
                                        &wgpu::CommandEncoderDescriptor {
                                            label: Some("lines_encoder"),
                                        },
                                    );
                                    let gizmo_origin = selected_id.and_then(|id| {
                                        entities
                                            .iter()
                                            .find(|(eid, _)| *eid == id)
                                            .map(|(_, t)| t.translation)
                                    });
                                    let cell_size = ui_ctx
                                        .world_session
                                        .lock()
                                        .as_ref()
                                        .map(|s| s.manifest.cell_size)
                                        .unwrap_or(16.0);
                                    let mut overlay =
                                        build_editor_overlay_lines(gizmo_origin, cell_size);
                                    if *ui_ctx.show_git_ghosts.lock() {
                                        let ghosts = ui_ctx.git.lock().ghosts.clone();
                                        let w = world.lock();
                                        for (id, old) in &ghosts {
                                            overlay.extend(ghost_cube_lines(
                                                old.translation,
                                                [1.0, 0.85, 0.2, 0.7],
                                            ));
                                            for e in w.all_entities() {
                                                if w.get_component::<redox_scene::StreamedActor>(e)
                                                    .is_some_and(|a| a.id == *id)
                                                {
                                                    if let Some(t) = w.get_component::<Transform>(e)
                                                    {
                                                        overlay.push(ghost_move_line(
                                                            old.translation,
                                                            t.translation,
                                                        ));
                                                    }
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    lp.render(
                                        &mut encoder,
                                        &host.viewport,
                                        host.queue(),
                                        &cam,
                                        &overlay,
                                    );
                                    host.queue().submit(Some(encoder.finish()));
                                }
                        }

                        let raw_input = egui_winit.take_egui_input(&window);
                        let ui_ctx_ref = ui_ctx.clone();
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            // Ctrl+P palette
                            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::P)) {
                                ui_ctx_ref.palette.lock().toggle();
                            }

                            egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
                                ui.horizontal(|ui| {
                                    ui.menu_button("File", |ui| {
                                        if ui.button("New World").clicked()
                                            && let Ok(s) = WorldSession::create_new(
                                                "worlds/untitled",
                                                "empty",
                                                4,
                                                4,
                                            ) {
                                                *ui_ctx_ref.world_session.lock() = Some(s);
                                                ui_ctx_ref.log("[INFO] Created worlds/untitled");
                                            }
                                        if ui.button("Open worlds/arena").clicked()
                                            && let Ok(s) = WorldSession::open("worlds/arena") {
                                                *ui_ctx_ref.world_session.lock() = Some(s);
                                                ui_ctx_ref.log("[INFO] Opened worlds/arena");
                                            }
                                        if ui.button("Save Layout Ctrl+S").clicked() {
                                            let _ = dock.save_to_file(std::path::Path::new(
                                                "editor_layout.ron",
                                            ));
                                        }
                                    });
                                    ui.menu_button("Edit", |ui| {
                                        if ui.button("Undo Ctrl+Z").clicked() {
                                            let mut w = world.lock();
                                            let r = registry.lock();
                                            undo_stack.lock().undo(&mut w, &r);
                                        }
                                        if ui.button("Redo Ctrl+Y").clicked() {
                                            let mut w = world.lock();
                                            let r = registry.lock();
                                            undo_stack.lock().redo(&mut w, &r);
                                        }
                                        if ui.button("Command Palette Ctrl+P").clicked() {
                                            ui_ctx_ref.palette.lock().toggle();
                                        }
                                    });
                                    ui.menu_button("Entity", |ui| {
                                        if ui.button("Create Empty Ctrl+Shift+N").clicked() {
                                            run_command(&ui_ctx_ref, &mut dock, "create_entity");
                                        }
                                        if ui.button("Duplicate Ctrl+D").clicked() {
                                            run_command(&ui_ctx_ref, &mut dock, "duplicate");
                                        }
                                        if ui.button("Delete").clicked() {
                                            run_command(&ui_ctx_ref, &mut dock, "delete");
                                        }
                                    });
                                    ui.menu_button("World", |ui| {
                                        // Agent E: list worlds/ via suggest_world_roots → open_world:
                                        let roots = WorldSession::suggest_world_roots();
                                        if roots.is_empty() {
                                            ui.label("No worlds/ directories found");
                                        } else {
                                            for root in roots {
                                                let label = format!("Open {}", root.display());
                                                if ui.button(label).clicked() {
                                                    *ui_ctx_ref.status.lock() =
                                                        format!("open_world:{}", root.display());
                                                    ui.close_menu();
                                                }
                                            }
                                        }
                                        ui.separator();
                                        if ui.button("Tick Streaming").clicked()
                                            && let Some(session) =
                                                ui_ctx_ref.world_session.lock().as_mut()
                                            {
                                                let eye = camera.lock().eye;
                                                session.tick(&mut world.lock(), eye);
                                                ui_ctx_ref.log("[INFO] World partition tick");
                                            }
                                    });
                                    ui.menu_button("Git", |ui| {
                                        // Agent E: surface git2 availability + refresh
                                        let git = ui_ctx_ref.git.lock();
                                        ui.label(git.summary());
                                        ui.label(if git.is_available() {
                                            "editor-git: git2 compiled in"
                                        } else {
                                            "editor-git off — OFPA dirty tracking only"
                                        });
                                        drop(git);
                                        if ui.button("Refresh status").clicked() {
                                            *ui_ctx_ref.status.lock() = "git_refresh".into();
                                            ui.close_menu();
                                        }
                                    });
                                    ui.menu_button("View", |ui| {
                                        if ui.button("Focus Selection F").clicked() {
                                            focus_selection(&ui_ctx_ref);
                                        }
                                        if ui.button("Quad View (F5)").clicked() {
                                            let mut l = ui_ctx_ref.layout.lock();
                                            *l = l.toggle();
                                        }
                                        if ui.button("Reset Layout").clicked() {
                                            run_command(&ui_ctx_ref, &mut dock, "reset_layout");
                                        }
                                    });
                                    ui.menu_button("Build", |ui| {
                                        if ui.button("Bake Meshlets").clicked() {
                                            run_command(&ui_ctx_ref, &mut dock, "bake_meshlets");
                                        }
                                        if ui.button("Write Bake Report").clicked() {
                                            run_command(&ui_ctx_ref, &mut dock, "bake_report");
                                        }
                                        if ui.button("Spawn StateTree Agent").clicked() {
                                            run_command(&ui_ctx_ref, &mut dock, "spawn_agent");
                                        }
                                    });
                                    ui.menu_button("Help", |ui| {
                                        if ui.button("About").clicked() {
                                            ui_ctx_ref.log(
                                                "[INFO] RedOx Studio — MSAA+SMAA editor shell",
                                            );
                                        }
                                    });

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let pie = ui_ctx_ref.pie.lock();
                                            let running = pie.running;
                                            let paused = pie.paused;
                                            drop(pie);

                                            if ui.button("⏹").clicked() {
                                                run_command(&ui_ctx_ref, &mut dock, "pie_stop");
                                            }
                                            if ui.button("⏸").clicked() {
                                                let mut pie = ui_ctx_ref.pie.lock();
                                                if pie.running {
                                                    if pie.paused {
                                                        pie.resume();
                                                    } else {
                                                        pie.pause();
                                                    }
                                                }
                                            }
                                            let play_label = if running && !paused {
                                                "▶ Playing"
                                            } else if paused {
                                                "▶ Resume"
                                            } else {
                                                "▶ Play"
                                            };
                                            if ui.button(play_label).clicked() {
                                                run_command(&ui_ctx_ref, &mut dock, "pie_play");
                                            }
                                            ui.separator();
                                            let sel = selection.lock().primary.map(|e| e.id());
                                            ui.label(format!("Sel:{:?}", sel));
                                            ui.label(format!("Gizmo:{:?}", gizmo.lock().mode));
                                        },
                                    );
                                });
                            });

                            egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
                                ui.horizontal(|ui| {
                                    let git = if ui_ctx_ref.world_session.lock().is_some() {
                                        format!("🟢 {}", ui_ctx_ref.git.lock().summary())
                                    } else {
                                        "⚪ No world".into()
                                    };
                                    ui.label(git);
                                    ui.separator();
                                    let fps = *ui_ctx_ref.fps.lock();
                                    let ms = *ui_ctx_ref.frame_ms.lock();
                                    let low1 = ui_ctx_ref.profiler.lock().one_percent_low_fps();
                                    ui.label(format!("FPS: {fps:.0} ({ms:.1}ms) · 1% {low1:.0}"));
                                    ui.separator();
                                    ui.label(format!("{:?}", *ui_ctx_ref.shading.lock()));
                                    ui.separator();
                                    ui.label(ui_ctx_ref.vram_label.lock().clone());
                                    ui.separator();
                                    let cell = selection
                                        .lock()
                                        .primary
                                        .and_then(|e| {
                                            world.lock().get_component::<Transform>(e).map(|t| {
                                                format!(
                                                    "Cell: ({:.0},{:.0})",
                                                    (t.translation.x / 16.0).floor(),
                                                    (t.translation.z / 16.0).floor()
                                                )
                                            })
                                        })
                                        .unwrap_or_else(|| "Cell: (0,0)".into());
                                    ui.label(cell);
                                    ui.separator();
                                    ui.label(format!("Undo: {}", undo_stack.lock().undo.len()));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let snap = *ui_ctx_ref.surface_snap.lock();
                                            ui.label(if snap {
                                                "Surface Snap: ON"
                                            } else {
                                                "Surface Snap: OFF"
                                            });
                                        },
                                    );
                                });
                            });

                            dock.show_with_ctx(
                                ctx,
                                &ui_ctx_ref,
                                viewport_tex_id,
                                Some(viewport_rect.clone()),
                            );

                            if let Some(cmd) = ui_ctx_ref.palette.lock().show(ctx) {
                                run_command(&ui_ctx_ref, &mut dock, &cmd);
                            }
                        });

                        // Pick on click (CPU raycast — avoids broken GPU ID buffer / map crashes)
                        let is_click = egui_ctx.input(|i| i.pointer.primary_clicked());
                        let hover_pos = egui_ctx.input(|i| i.pointer.hover_pos());
                        let alt = egui_ctx.input(|i| i.modifiers.alt);
                        if is_click && !alt && gizmo.lock().axis.is_none()
                            && let (Some(pos), Some(rect), Some(host)) =
                                (hover_pos, *viewport_rect.lock(), render_host.as_ref())
                            {
                                let vp = &host.viewport;
                                let ppp = window.scale_factor() as f32;
                                if let Some((x, y)) =
                                    egui_to_viewport_coords(pos, rect, ppp, vp.size)
                                {
                                    let brush_on = ui_ctx.brush.lock().active;
                                    let uv = redox_math::Vec2::new(
                                        (x as f32 + 0.5) / vp.size.0 as f32,
                                        (y as f32 + 0.5) / vp.size.1 as f32,
                                    );
                                    let cam = camera.lock().clone();
                                    let layout = *ui_ctx.layout.lock();
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
                                                    let b = ui_ctx.brush.lock();
                                                    (b.settings.clone(), b.kind.clone())
                                                };
                                                let poses = scatter_poses(
                                                    hit,
                                                    settings.radius,
                                                    settings.density,
                                                    (hit.x * 1000.0) as u64
                                                        ^ (hit.z * 1000.0) as u64,
                                                );
                                                if let Some(session) =
                                                    ui_ctx.world_session.lock().as_mut()
                                                {
                                                    let n = session.paint_actors(
                                                        &mut world.lock(),
                                                        &poses,
                                                        &kind,
                                                    );
                                                    ui_ctx.log(format!(
                                                        "[INFO] Brush painted {n} × {kind} at ({:.1},{:.1})",
                                                        hit.x, hit.z
                                                    ));
                                                    ui_ctx
                                                        .git
                                                        .lock()
                                                        .refresh_from_world(&world.lock());
                                                } else {
                                                    let mut w = world.lock();
                                                    for (p, yaw) in poses {
                                                        let e = w.spawn();
                                                        w.add_component(
                                                            e,
                                                            Transform {
                                                                translation: p
                                                                    + Vec3::Y * 0.5,
                                                                rotation:
                                                                    redox_math::Quat::from_rotation_y(
                                                                        yaw,
                                                                    ),
                                                                scale: Vec3::ONE,
                                                            },
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    } else if let Some(id) =
                                        pick_unit_cubes(&entities, origin, dir)
                                    {
                                        if let Some(entity) = resolve_entity(&world.lock(), id) {
                                            selection.lock().set_primary(Some(entity));
                                            ui_ctx.log(format!(
                                                "[INFO] Picked entity #{} at ({x},{y})",
                                                entity.id()
                                            ));
                                        }
                                    } else {
                                        selection.lock().clear();
                                    }
                                }
                            }

                        {
                            let mut world_guard = world.lock();
                            let registry_guard = registry.lock();
                            let mut undo_guard = undo_stack.lock();
                            undo_guard.handle_hotkeys(&egui_ctx, &mut world_guard, &registry_guard);
                        }

                        egui_winit.handle_platform_output(&window, full_output.platform_output);

                        let host = render_host.as_ref().unwrap();
                        let dev = host.device();
                        let que = host.queue();

                        let screen_desc = egui_wgpu::ScreenDescriptor {
                            size_in_pixels: [cfg.width, cfg.height],
                            pixels_per_point: window.scale_factor() as f32,
                        };
                        for (id, image_delta) in &full_output.textures_delta.set {
                            renderer.update_texture(dev, que, *id, image_delta);
                        }

                        let tris = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
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
                        window.request_redraw();
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    });
}
