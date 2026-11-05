//! Editor commands (palette / menu / hotkeys) and small world helpers.

use redox_ecs::{Entity, World};
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
pub(crate) fn run_command(ui_ctx: &EditorUiCtx, dock: &mut EditorDock, id: &str) {
    if ui_ctx.core.commands.contains(id) {
        ui_ctx.core.commands.run(&ui_ctx.core, id);
        return;
    }
    match id {
        "focus" => focus_selection(ui_ctx),
        "reset_layout" => {
            dock.reset_layout();
            ui_ctx.log("[INFO] Layout reset");
        }
        "pie_play" => {
            let mut pie = ui_ctx.pie.lock();
            let world = ui_ctx.core.world.lock();
            let registry = ui_ctx.core.registry.lock();
            if pie.running && pie.paused {
                pie.resume();
            } else if !pie.running {
                pie.start(&world, &registry);
                ui_ctx.log("[INFO] PIE started");
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
        "align_surface" => {
            if let Some(e) = ui_ctx.core.selection.lock().primary {
                *ui_ctx.core.status.lock() = format!("snap:{}", e.id());
            }
        }
        other => ui_ctx.log(format!("[WARN] Unknown command: {other}")),
    }
}
