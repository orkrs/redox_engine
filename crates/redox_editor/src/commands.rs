//! Editor commands (palette / menu / hotkeys) and small world helpers.

use redox_ecs::{Entity, World};
use redox_editor_core::CommandRegistry;
use redox_editor_debugger::{PieRunMode, plan_standalone, prepare_pie_world};
use redox_editor_plugins::PluginHost;
use redox_editor_ui::{EditorDock, EditorUiCtx};
use redox_editor_viewport::ViewportShading;
use redox_math::{Transform, Vec3};

pub(crate) fn resolve_entity(world: &World, id: u32) -> Option<Entity> {
    world.all_entities().find(|e| e.id() == id)
}

pub(crate) fn collect_transforms(world: &World) -> Vec<(u32, Transform)> {
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

pub(crate) fn focus_selection(ui_ctx: &EditorUiCtx) {
    let sel = ui_ctx.core.selection.lock().primary;
    if let Some(e) = sel
        && let Some(t) = ui_ctx
            .core
            .world
            .lock()
            .get_component::<Transform>(e)
            .copied()
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

/// Runs the command `id`. Commands registered in `EditorContext` run first; the remaining
/// identifiers need viewport, debugger or dock state and are handled here.
pub(crate) fn run_command(
    ui_ctx: &EditorUiCtx,
    dock: &mut EditorDock,
    plugin_host: Option<&mut PluginHost>,
    commands: &mut CommandRegistry,
    isolate_plugin_panics: bool,
    id: &str,
) {
    if commands.contains(id) {
        if isolate_plugin_panics {
            if let Some(host) = plugin_host.filter(|host| host.owns_command(id)) {
                host.run_command(commands, &ui_ctx.core, id);
                return;
            }
        }
        commands.run(&ui_ctx.core, id);
        return;
    }
    match id {
        "focus" => focus_selection(ui_ctx),
        "reset_layout" => {
            dock.reset_layout();
            ui_ctx.log("[INFO] Layout reset");
        }
        "pie_play" => {
            let settings = ui_ctx.validation.lock().settings;
            let report = ui_ctx.validation.lock().report.clone();
            if redox_editor_ui::file_menu::FileMenu::pie_blocked(&report, &settings) {
                ui_ctx.log("[WARN] PIE blocked: fix validation errors first");
                return;
            }
            let mut pie = ui_ctx.pie.lock();
            let mut world = ui_ctx.core.world.lock();
            let registry = ui_ctx.core.registry.lock();
            if pie.running && pie.paused {
                pie.resume();
            } else if !pie.running {
                let settings = redox_editor_core::settings::EditorSettings::default().pie;
                let project_root = ui_ctx
                    .core
                    .status
                    .lock()
                    .strip_prefix("project:")
                    .map(std::path::Path::new);
                prepare_pie_world(&settings, project_root, &registry, &mut world);
                pie.start_with_mode(&world, &registry, PieRunMode::Play);
                ui_ctx.log("[INFO] PIE started");
            }
        }
        "pie_simulate" => {
            let mut pie = ui_ctx.pie.lock();
            let world = ui_ctx.core.world.lock();
            let registry = ui_ctx.core.registry.lock();
            if !pie.running {
                pie.start_with_mode(&world, &registry, PieRunMode::Simulate);
                ui_ctx.log("[INFO] PIE simulate started");
            }
        }
        "pie_step" => {
            let mut pie = ui_ctx.pie.lock();
            let mut world = ui_ctx.core.world.lock();
            let registry = ui_ctx.core.registry.lock();
            if !pie.running {
                pie.start_with_mode(&world, &registry, PieRunMode::Simulate);
            }
            pie.step(&mut world, &registry, &mut |_, _| {});
            ui_ctx.log(format!("[INFO] PIE step (frame {})", pie.frame_count));
        }
        "pie_standalone" => {
            let project = ui_ctx
                .core
                .status
                .lock()
                .strip_prefix("project:")
                .map(str::to_owned)
                .unwrap_or_else(|| ".".into());
            let plan = plan_standalone(std::path::Path::new(&project), None);
            match plan {
                redox_editor_debugger::StandalonePlan::EditorPlayHost { project } => {
                    ui_ctx.log(format!(
                        "[INFO] Standalone: redox_editor --play {}",
                        project.display()
                    ));
                    *ui_ctx.core.status.lock() = format!("standalone:{}", project.display());
                }
                redox_editor_debugger::StandalonePlan::Unavailable(reason) => {
                    ui_ctx.log(format!("[WARN] Standalone unavailable: {reason}"));
                }
            }
        }
        "pie_stop" => {
            let mut pie = ui_ctx.pie.lock();
            let mut world = ui_ctx.core.world.lock();
            let registry = ui_ctx.core.registry.lock();
            pie.stop(&mut world, &registry);
            ui_ctx.time_travel.lock().frames.clear();
            ui_ctx.log("[INFO] PIE stopped — world restored");
        }
        "bake_meshlets" => {
            ui_ctx.log("[WARN] Bake Meshlets not wired yet — use Packager for OFPA report")
        }
        "bake_navmesh" => {
            ui_ctx.log("[INFO] Use Packager → Bake Navmesh (same path as redox_cook)")
        }
        "bake_report" => *ui_ctx.core.status.lock() = "bake_report".into(),
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
        "shading_roughness" => {
            *ui_ctx.shading.lock() = ViewportShading::Roughness;
            ui_ctx.log("[INFO] Shading: Roughness");
        }
        "shading_depth" => {
            *ui_ctx.shading.lock() = ViewportShading::Depth;
            ui_ctx.log("[INFO] Shading: Depth");
        }
        "viewport_screenshot" => {
            *ui_ctx.core.status.lock() = "viewport_screenshot".into();
            ui_ctx.log("[INFO] Viewport screenshot queued");
        }
        "viewport_toggle_sky" => {
            *ui_ctx.core.status.lock() = "viewport_toggle:procedural_sky".into();
        }
        "viewport_toggle_gi" => {
            *ui_ctx.core.status.lock() = "viewport_toggle:gi_enabled".into();
        }
        "viewport_toggle_fog" => {
            *ui_ctx.core.status.lock() = "viewport_toggle:fog_enabled".into();
        }
        "viewport_toggle_sdf_debug" => {
            *ui_ctx.core.status.lock() = "viewport_toggle:sdf_debug".into();
        }
        "align_surface" => {
            if let Some(e) = ui_ctx.core.selection.lock().primary {
                *ui_ctx.core.status.lock() = format!("snap:{}", e.id());
            }
        }
        "material_save" => {
            let mut material = ui_ctx.material.lock();
            if let Some(session) = material.as_mut() {
                match session.save() {
                    Ok(path) => ui_ctx.log(format!("[INFO] Saved material {path}")),
                    Err(err) => ui_ctx.log(format!("[ERROR] Material save failed: {err}")),
                }
            } else {
                ui_ctx.log("[WARN] No material document open");
            }
        }
        "graph_tab" => {
            ui_ctx.log("[INFO] Open the Graph dock tab to edit materials");
            *ui_ctx.core.status.lock() = "focus_graph_tab".into();
        }
        other => ui_ctx.log(format!("[WARN] Unknown command: {other}")),
    }
}
