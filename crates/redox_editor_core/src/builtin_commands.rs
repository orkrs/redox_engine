//! Commands that work on the domain state (world, selection, log).
//!
//! Each command that changes the world runs as one named transaction of the undo history.

use crate::command::EditorCommand;
use crate::context::EditorContext;
use crate::guid::GuidResolver;
use crate::undo::{UndoCommand, capture_entity};
use redox_ecs::Entity;
use redox_math::Transform;
use redox_reflect::EntitySnapshot;

/// Records the creation of `entity` in the open transaction. The entity and its components must
/// already be in the world; the caller opens and commits the transaction.
pub fn record_spawn(ctx: &EditorContext, entity: Entity) {
    let guid = GuidResolver::ensure(&mut ctx.world.lock(), entity);
    let snapshot = snapshot_of(ctx, entity);
    ctx.undo.lock().push(UndoCommand::Spawn { guid, snapshot });
}

/// Captures the registered components of `entity`.
fn snapshot_of(ctx: &EditorContext, entity: Entity) -> EntitySnapshot {
    let world = ctx.world.lock();
    let registry = ctx.registry.lock();
    capture_entity(&world, entity, &registry)
}

/// Spawns an entity with `transform`; the caller records it.
fn spawn_with_transform(ctx: &EditorContext, transform: Transform) -> Entity {
    let mut world = ctx.world.lock();
    let entity = world.spawn(); // undo: tx
    world.add_component(entity, transform); // undo: tx
    entity
}

/// Spawns an entity with a default transform and selects it.
pub struct CreateEntity;

impl EditorCommand for CreateEntity {
    fn id(&self) -> &str {
        "create_entity"
    }

    fn label(&self) -> &str {
        "> Create Empty Entity"
    }

    fn execute(&self, ctx: &EditorContext) {
        ctx.undo.lock().begin("Create Entity");
        let entity = spawn_with_transform(ctx, Transform::default());
        record_spawn(ctx, entity);
        ctx.undo.lock().commit();
        ctx.selection.lock().set_primary(Some(entity));
        ctx.log.info(format!("Created entity #{}", entity.id()));
    }
}

/// Copies the selected entity's transform one unit along X into a new entity and selects it.
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
        ctx.undo.lock().begin("Duplicate Entity");
        let copy = spawn_with_transform(ctx, copy_transform);
        record_spawn(ctx, copy);
        ctx.undo.lock().commit();
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
        ctx.undo.lock().begin("Delete Entity");
        let guid = GuidResolver::ensure(&mut ctx.world.lock(), entity);
        let snapshot = snapshot_of(ctx, entity);
        ctx.undo.lock().push(UndoCommand::Despawn { guid, snapshot });
        ctx.world.lock().despawn(entity); // undo: tx
        ctx.undo.lock().commit();
        ctx.selection.lock().clear();
        ctx.log.info(format!("Deleted entity #{}", entity.id()));
    }
}
