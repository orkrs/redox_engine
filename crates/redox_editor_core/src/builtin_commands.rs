//! Commands that only touch the domain state (world, selection, log).
//!
//! They do not record Undo steps yet; that is the job of a later slice.

use crate::command::EditorCommand;
use crate::context::EditorContext;
use redox_math::Transform;
use redox_scene::EntityGuid;

/// Spawns an entity with a default transform and a new guid, and selects it.
pub struct CreateEntity;

impl EditorCommand for CreateEntity {
    fn id(&self) -> &str {
        "create_entity"
    }

    fn label(&self) -> &str {
        "> Create Empty Entity"
    }

    fn execute(&self, ctx: &EditorContext) {
        let entity = {
            let mut world = ctx.world.lock();
            let entity = world.spawn();
            world.add_component(entity, Transform::default());
            world.add_component(entity, EntityGuid::new_random());
            entity
        };
        ctx.selection.lock().set_primary(Some(entity));
        ctx.log.info(format!("Created entity #{}", entity.id()));
    }
}

/// Copies the selected entity's transform one unit along X into a new entity with a new guid
/// (the copy never shares the source's identity) and selects it.
pub struct DuplicateEntity;

impl EditorCommand for DuplicateEntity {
    fn id(&self) -> &str {
        "duplicate"
    }

    fn label(&self) -> &str {
        "> Duplicate Entity"
    }

    fn execute(&self, ctx: &EditorContext) {
        let primary = ctx.selection.lock().primary;
        let Some(source) = primary else {
            return;
        };
        let source_transform = ctx
            .world
            .lock()
            .get_component::<Transform>(source)
            .copied()
            .unwrap_or_default();
        let mut copy_transform = source_transform;
        copy_transform.translation.x += 1.0;
        let copy = {
            let mut world = ctx.world.lock();
            let copy = world.spawn();
            world.add_component(copy, copy_transform);
            world.add_component(copy, EntityGuid::new_random());
            copy
        };
        ctx.selection.lock().set_primary(Some(copy));
        ctx.log
            .info(format!("Duplicated #{} → #{}", source.id(), copy.id()));
    }
}

/// Despawns the primary selected entity and clears the selection.
pub struct DeleteEntity;

impl EditorCommand for DeleteEntity {
    fn id(&self) -> &str {
        "delete"
    }

    fn label(&self) -> &str {
        "> Delete Entity"
    }

    fn execute(&self, ctx: &EditorContext) {
        let primary = ctx.selection.lock().primary.take();
        let Some(entity) = primary else {
            return;
        };
        ctx.world.lock().despawn(entity);
        ctx.selection.lock().clear();
        ctx.log.info(format!("Deleted entity #{}", entity.id()));
    }
}
