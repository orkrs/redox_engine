//! Scene outliner tree: build from the ECS world, filter, flatten for UI and virtual scroll.

use std::collections::{HashMap, HashSet, VecDeque};

use redox_ecs::hierarchy::Parent;
use redox_ecs::{Entity, World};
use redox_editor_core::editor_only::EditorOnly;
use redox_editor_core::markers::EditorFolder;
use redox_editor_core::selection::{Selection, Visibility};
use redox_editor_core::markers::Locked;
use redox_reflect::{ReflectValue, TypeRegistry};
use redox_scene::{EntityGuid, Name};

use crate::guid_index::GuidIndex;
use crate::outliner_ops::{is_hidden, is_locked};

/// Default row height (pixels) used by the virtual list and tests.
pub const OUTLINER_ROW_HEIGHT: f32 = 20.0;

/// One node of the hierarchy shown in the outliner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerNode {
    /// Stable entity id.
    pub guid: EntityGuid,
    /// Display label.
    pub name: String,
    /// Parent guid when linked.
    pub parent: Option<EntityGuid>,
    /// Direct children in build order.
    pub children: Vec<EntityGuid>,
    /// Entity hidden from the viewport.
    pub hidden: bool,
    /// Entity locked against edits.
    pub locked: bool,
    /// Validation flagged this guid.
    pub has_error: bool,
    /// Outliner folder marker.
    pub is_folder: bool,
}

/// User-facing problem while building the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerWarning {
    /// Affected entity.
    pub guid: EntityGuid,
    /// Short explanation.
    pub message: String,
}

/// Text filters applied when flattening the tree.
#[derive(Clone, Debug, Default)]
pub struct OutlinerFilter {
    /// Substring match against the entity name (case insensitive).
    pub name: String,
    /// Substring match against a registered component type name.
    pub component: String,
    /// When set, only entities with validation errors (and their ancestors) stay visible.
    pub errors_only: bool,
}

/// Built hierarchy ready for flattening.
#[derive(Clone, Debug, Default)]
pub struct OutlinerModel {
    /// Root guids in display order.
    pub roots: Vec<EntityGuid>,
    /// All indexed nodes.
    pub nodes: HashMap<EntityGuid, OutlinerNode>,
    /// Problems collected while building.
    pub warnings: Vec<OutlinerWarning>,
}

/// One visible row after filters and expand/collapse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatRow {
    /// Entity id for the row.
    pub guid: EntityGuid,
    /// Indent depth (0 = root).
    pub depth: usize,
    /// Whether the node has children (for twisty).
    pub has_children: bool,
}

/// Result of [`visible_row_range`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleRows {
    /// Indices into the flat list to draw this frame.
    pub range: std::ops::Range<usize>,
    /// Number of rows that would be drawn (equals `range` length).
    pub rows_drawn: usize,
}

/// Builds the hierarchy from `world` and `index`.
pub fn build(world: &World, index: &GuidIndex, error_guids: &HashSet<EntityGuid>) -> OutlinerModel {
    let mut model = OutlinerModel::default();
    let mut raw_parent: HashMap<EntityGuid, Option<EntityGuid>> = HashMap::new();

    for entity in world.all_entities() {
        if world.get_component::<EditorOnly>(entity).is_some() {
            continue;
        }
        let Some(guid) = index.guid_of(entity) else {
            continue;
        };
        let name = world
            .get_component::<Name>(entity)
            .map(|n| n.0.clone())
            .unwrap_or_else(|| format!("Entity {}", guid));
        let parent = world
            .get_component::<Parent>(entity)
            .and_then(|p| index.guid_of(p.0));
        raw_parent.insert(guid, parent);
        model.nodes.insert(
            guid,
            OutlinerNode {
                guid,
                name,
                parent: None,
                children: Vec::new(),
                hidden: is_hidden(world, entity),
                locked: is_locked(world, entity),
                has_error: error_guids.contains(&guid),
                is_folder: world.get_component::<EditorFolder>(entity).is_some(),
            },
        );
    }

    for (&guid, &parent) in &raw_parent {
        let parent = match parent {
            None => {
                model.roots.push(guid);
                continue;
            }
            Some(p) => p,
        };
        if !model.nodes.contains_key(&parent) {
            model.warnings.push(OutlinerWarning {
                guid,
                message: format!("missing parent {parent}; placed at root"),
            });
            model.roots.push(guid);
            continue;
        }
        if parent == guid || parent_chain_cycle(&raw_parent, guid, parent) {
            model.warnings.push(OutlinerWarning {
                guid,
                message: "parent cycle; placed at root".into(),
            });
            model.roots.push(guid);
            continue;
        }
        if let Some(node) = model.nodes.get_mut(&guid) {
            node.parent = Some(parent);
        }
        if let Some(parent_node) = model.nodes.get_mut(&parent) {
            parent_node.children.push(guid);
        }
    }

    model.roots.sort_by_key(|g| model.nodes.get(g).map(|n| n.name.clone()));
    for node in model.nodes.values_mut() {
        node.children.sort_by_key(|g| model.nodes.get(g).map(|n| n.name.clone()));
    }
    model
}

fn parent_chain_cycle(
    raw: &HashMap<EntityGuid, Option<EntityGuid>>,
    child: EntityGuid,
    mut current: EntityGuid,
) -> bool {
    let mut seen = HashSet::new();
    seen.insert(child);
    while let Some(&Some(parent)) = raw.get(&current) {
        if parent == child {
            return true;
        }
        if !seen.insert(parent) {
            return true;
        }
        current = parent;
    }
    false
}

/// Depth-first visible rows with filters and expansion.
pub fn flatten(
    model: &OutlinerModel,
    world: &World,
    registry: &TypeRegistry,
    filter: &OutlinerFilter,
    error_guids: &HashSet<EntityGuid>,
    expanded: &HashSet<EntityGuid>,
) -> Vec<FlatRow> {
    let visible = visible_guids(model, world, registry, filter, error_guids);
    let mut out = Vec::new();
    for &root in &model.roots {
        walk_flat(
            model,
            root,
            0,
            expanded,
            &visible,
            &mut out,
        );
    }
    out
}

fn visible_guids(
    model: &OutlinerModel,
    world: &World,
    registry: &TypeRegistry,
    filter: &OutlinerFilter,
    error_guids: &HashSet<EntityGuid>,
) -> HashSet<EntityGuid> {
    if filter.name.is_empty() && filter.component.is_empty() && !filter.errors_only {
        return model.nodes.keys().copied().collect();
    }
    let name_needle = filter.name.to_lowercase();
    let component_needle = filter.component.to_lowercase();
    let mut matched = HashSet::new();
    for (guid, node) in &model.nodes {
        if filter.errors_only && !error_guids.contains(guid) {
            continue;
        }
        let name_ok = name_needle.is_empty() || node.name.to_lowercase().contains(&name_needle);
        let entity = entity_for_guid(world, *guid);
        let component_ok = component_needle.is_empty()
            || entity.is_some_and(|e| entity_has_component(world, registry, e, &component_needle));
        if name_ok && component_ok {
            matched.insert(*guid);
        }
    }
    let mut visible = HashSet::new();
    for guid in matched {
        visible.insert(guid);
        add_ancestors(model, guid, &mut visible);
        add_descendants(model, guid, &mut visible);
    }
    visible
}

fn entity_for_guid(world: &World, guid: EntityGuid) -> Option<Entity> {
    world.all_entities().find(|e| {
        world
            .get_component::<EntityGuid>(*e)
            .is_some_and(|g| *g == guid)
    })
}

fn add_ancestors(model: &OutlinerModel, mut guid: EntityGuid, out: &mut HashSet<EntityGuid>) {
    while let Some(parent) = model.nodes.get(&guid).and_then(|n| n.parent) {
        out.insert(parent);
        guid = parent;
    }
}

fn add_descendants(model: &OutlinerModel, guid: EntityGuid, out: &mut HashSet<EntityGuid>) {
    let mut queue = VecDeque::from([guid]);
    while let Some(current) = queue.pop_front() {
        if let Some(node) = model.nodes.get(&current) {
            for &child in &node.children {
                if out.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }
}

fn entity_has_component(
    world: &World,
    registry: &TypeRegistry,
    entity: Entity,
    needle: &str,
) -> bool {
    registry
        .snapshot_entity(world, entity)
        .into_iter()
        .any(|value| {
            matches!(value, ReflectValue::Struct { type_name, .. }
                if type_name.to_lowercase().contains(needle))
        })
}

fn walk_flat(
    model: &OutlinerModel,
    guid: EntityGuid,
    depth: usize,
    expanded: &HashSet<EntityGuid>,
    visible: &HashSet<EntityGuid>,
    out: &mut Vec<FlatRow>,
) {
    if !visible.contains(&guid) {
        return;
    }
    let Some(node) = model.nodes.get(&guid) else {
        return;
    };
    let has_children = !node.children.is_empty();
    out.push(FlatRow {
        guid,
        depth,
        has_children,
    });
    let is_open = expanded.is_empty() || expanded.contains(&guid);
    if !is_open {
        return;
    }
    for &child in &node.children {
        walk_flat(model, child, depth + 1, expanded, visible, out);
    }
}

/// Rows that fit in the viewport; used by the UI and performance tests.
pub fn visible_row_range(
    total_rows: usize,
    scroll_offset_y: f32,
    viewport_height: f32,
    row_height: f32,
) -> VisibleRows {
    if total_rows == 0 || viewport_height <= 0.0 {
        return VisibleRows {
            range: 0..0,
            rows_drawn: 0,
        };
    }
    let first = (scroll_offset_y / row_height).floor().max(0.0) as usize;
    let count = (viewport_height / row_height).ceil() as usize + 1;
    let last = (first + count).min(total_rows);
    let range = first..last;
    VisibleRows {
        rows_drawn: range.len(),
        range,
    }
}

/// Shift-click range selection along the visible flat list.
pub fn select_range(
    selection: &mut Selection,
    flat: &[FlatRow],
    index: &GuidIndex,
    anchor: EntityGuid,
    target: EntityGuid,
) {
    let Some(a) = flat.iter().position(|r| r.guid == anchor) else {
        return;
    };
    let Some(b) = flat.iter().position(|r| r.guid == target) else {
        return;
    };
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    let entities: Vec<Entity> = flat[start..=end]
        .iter()
        .filter_map(|row| index.get(row.guid))
        .collect();
    selection.replace(entities);
}

/// Ctrl-style toggle with optional shift anchor stored by the caller.
pub fn toggle_select(selection: &mut Selection, entity: Entity, modifiers: SelectionModifiers) {
    if modifiers.shift {
        // Range needs anchor; no-op here when anchor missing.
        return;
    }
    if modifiers.ctrl {
        selection.toggle(entity);
    } else {
        selection.set_primary(Some(entity));
    }
}

/// Modifier keys for outliner clicks.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectionModifiers {
    /// Ctrl / Cmd held.
    pub ctrl: bool,
    /// Shift held.
    pub shift: bool,
}

/// Adds every descendant of `guid` to the selection.
pub fn select_children(
    selection: &mut Selection,
    model: &OutlinerModel,
    index: &GuidIndex,
    guid: EntityGuid,
    recursive: bool,
) {
    let mut queue = VecDeque::from([guid]);
    while let Some(current) = queue.pop_front() {
        let Some(node) = model.nodes.get(&current) else {
            continue;
        };
        for &child in &node.children {
            if let Some(entity) = index.get(child) {
                selection.add(entity);
            }
            if recursive {
                queue.push_back(child);
            }
        }
    }
}

/// Inverts selection along the visible flat list.
pub fn invert_selection(selection: &mut Selection, flat: &[FlatRow], index: &GuidIndex) {
    let mut next = HashSet::new();
    for row in flat {
        if let Some(entity) = index.get(row.guid)
            && !selection.set.contains(&entity)
        {
            next.insert(entity);
        }
    }
    let primary = next.iter().next().copied();
    selection.set.clear();
    selection.primary = primary;
    for e in next {
        selection.set.insert(e);
    }
}

use redox_math::Transform;

/// Spawns a small test hierarchy A → B → C with stable guids (tests only).
#[doc(hidden)]
pub fn spawn_chain_world(world: &mut World) -> (EntityGuid, EntityGuid, EntityGuid) {
    let ga = EntityGuid(1);
    let gb = EntityGuid(2);
    let gc = EntityGuid(3);
    let a = world.spawn();
    world.add_component(a, ga);
    world.add_component(a, Name("A".into()));
    world.add_component(a, Transform::default());
    let b = world.spawn();
    world.add_component(b, gb);
    world.add_component(b, Name("B".into()));
    world.add_component(b, Transform::default());
    let c = world.spawn();
    world.add_component(c, gc);
    world.add_component(c, Name("C".into()));
    world.add_component(c, Transform::default());
    redox_ecs::hierarchy::set_parent(world, b, a);
    redox_ecs::hierarchy::set_parent(world, c, b);
    (ga, gb, gc)
}

/// Spawns `count` flat entities for stress tests.
#[doc(hidden)]
pub fn spawn_many_entities(world: &mut World, count: usize) -> GuidIndex {
    let mut index = GuidIndex::new();
    for i in 0..count {
        let e = world.spawn();
        let guid = EntityGuid(i as u128 + 10_000);
        world.add_component(e, guid);
        world.add_component(e, Name(format!("Node {i}")));
        world.add_component(e, Transform::default());
        index.insert(guid, e);
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_editor_core::undo::{UndoCommand, UndoStack};
    use redox_math::{Transform, Vec3};
    use redox_reflect::TypeRegistry;
    use redox_render::Camera;
    use crate::register_scene_components;

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_scene_components(&mut registry);
        registry.register::<Visibility>();
        registry.register::<Locked>();
        registry.register::<EditorFolder>();
        registry
    }

    #[test]
    fn tree_builds_a_b_c_chain() {
        let mut world = World::new();
        let (a, b, c) = spawn_chain_world(&mut world);
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let model = build(&world, &index, &HashSet::new());
        assert_eq!(model.roots, vec![a]);
        assert_eq!(model.nodes[&b].parent, Some(a));
        assert_eq!(model.nodes[&c].parent, Some(b));
        let flat = flatten(
            &model,
            &world,
            &registry(),
            &OutlinerFilter::default(),
            &HashSet::new(),
            &HashSet::from([a, b]),
        );
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[2].guid, c);
    }

    #[test]
    fn name_filter_keeps_ancestors() {
        let mut world = World::new();
        let (a, _, c) = spawn_chain_world(&mut world);
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let model = build(&world, &index, &HashSet::new());
        let filter = OutlinerFilter {
            name: "c".into(),
            ..OutlinerFilter::default()
        };
        let flat = flatten(
            &model,
            &world,
            &registry(),
            &filter,
            &HashSet::new(),
            &HashSet::from([a]),
        );
        assert_eq!(flat.len(), 3, "ancestor path stays visible");
        assert!(flat.iter().any(|r| r.guid == c));
    }

    #[test]
    fn component_filter_matches_camera() {
        let mut world = World::new();
        let (a, _, _) = spawn_chain_world(&mut world);
        let entity = world
            .all_entities()
            .find(|e| world.get_component::<EntityGuid>(*e).is_some_and(|g| *g == a))
            .expect("entity");
        world.add_component(entity, Camera::default());
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let model = build(&world, &index, &HashSet::new());
        let filter = OutlinerFilter {
            component: "Camera".into(),
            ..OutlinerFilter::default()
        };
        let flat = flatten(
            &model,
            &world,
            &registry(),
            &filter,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].guid, a);
    }

    #[test]
    fn parent_cycle_becomes_root_with_warning() {
        let mut world = World::new();
        let g1 = EntityGuid(1);
        let g2 = EntityGuid(2);
        let e1 = world.spawn();
        let e2 = world.spawn();
        world.add_component(e1, g1);
        world.add_component(e2, g2);
        world.add_component(e1, Name("One".into()));
        world.add_component(e2, Name("Two".into()));
        redox_ecs::hierarchy::set_parent(world, e1, e2);
        redox_ecs::hierarchy::set_parent(world, e2, e1);
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let model = build(&world, &index, &HashSet::new());
        assert_eq!(model.roots.len(), 2);
        assert!(model
            .warnings
            .iter()
            .any(|w| w.message.contains("cycle")));
    }

    #[test]
    fn reparent_keep_world_position() {
        let mut world = World::new();
        let mut registry = registry();
        registry.register::<Transform>();
        let child_g = EntityGuid(10);
        let parent_g = EntityGuid(20);
        let child = world.spawn();
        let parent = world.spawn();
        world.add_component(child, child_g);
        world.add_component(parent, parent_g);
        world.add_component(
            child,
            Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
        );
        world.add_component(parent, Transform::default());
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let before_world = child_world_pos(&world, child);
        let cmd = UndoCommand::reparent(&world, child_g, Some(parent_g), true).expect("reparent");
        let mut stack = UndoStack::default();
        stack.begin("Reparent");
        cmd.clone().apply(&mut world, &registry).expect("apply");
        stack.push(cmd);
        stack.commit();
        let after_world = child_world_pos(&world, child);
        assert!((before_world - after_world).length() < 1e-4);
    }

    #[test]
    fn reparent_into_descendant_fails_without_world_change() {
        let mut world = World::new();
        let registry = registry();
        let (a, b, _) = spawn_chain_world(&mut world);
        let before = snapshot_parents(&world);
        let err = UndoCommand::reparent(&world, a, Some(b), true);
        assert!(matches!(err, Err(redox_editor_core::undo::UndoError::Cycle)));
        assert_eq!(snapshot_parents(&world), before);
        let _ = err;
    }

    #[test]
    fn undo_reparent_restores_hierarchy() {
        let mut world = World::new();
        let mut registry = registry();
        let (a, b, c) = spawn_chain_world(&mut world);
        let mut stack = UndoStack::default();
        let cmd = UndoCommand::reparent(&world, c, Some(a), true).expect("plan");
        stack.begin("Reparent");
        cmd.apply(&mut world, &registry).expect("apply");
        stack.push(cmd);
        stack.commit();
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let model = build(&world, &index, &HashSet::new());
        assert_eq!(model.nodes[&c].parent, Some(a));
        assert!(stack.undo(&mut world, &registry));
        let model = build(&world, &index, &HashSet::new());
        assert_eq!(model.nodes[&c].parent, Some(b));
    }

    #[test]
    fn shift_range_selects_n_nodes() {
        let mut world = World::new();
        let (a, b, c) = spawn_chain_world(&mut world);
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let model = build(&world, &index, &HashSet::new());
        let flat = flatten(
            &model,
            &world,
            &registry(),
            &OutlinerFilter::default(),
            &HashSet::new(),
            &HashSet::from([a]),
        );
        let mut selection = Selection::default();
        select_range(&mut selection, &flat, &index, a, c);
        assert_eq!(selection.set.len(), 3);
        let _ = b;
    }

    #[test]
    fn hide_makes_entity_not_pickable() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, EntityGuid(1));
        world.add_component(e, Visibility(false));
        assert!(!crate::outliner_ops::is_pickable(&world, e));
    }

    #[test]
    fn undo_isolate_restores_visibility() {
        let mut world = World::new();
        let mut registry = registry();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, EntityGuid(1));
        world.add_component(b, EntityGuid(2));
        let mut index = GuidIndex::new();
        index.rebuild(&world);
        let mut stack = UndoStack::default();
        let mut selected = HashSet::new();
        selected.insert(a);
        crate::outliner_ops::isolate(&mut stack, &mut world, &registry, &index, &selected)
            .expect("isolate");
        assert!(is_hidden(&world, b));
        assert!(stack.undo(&mut world, &registry));
        assert!(!is_hidden(&world, b));
    }

    #[test]
    fn virtual_list_does_not_draw_all_rows_at_50k() {
        let count = 50_000;
        let visible = visible_row_range(count, 0.0, 800.0, OUTLINER_ROW_HEIGHT);
        assert!(visible.rows_drawn < 100, "only a window of rows is drawn");
        assert_eq!(visible.range.len(), visible.rows_drawn);
    }

    #[test]
    #[ignore = "coordinator measures build time for 50k entities; see docs/BENCHMARKS.md"]
    fn bench_build_50k_entities() {
        let mut world = World::new();
        let index = spawn_many_entities(&mut world, 50_000);
        let _model = build(&world, &index, &HashSet::new());
    }

    fn child_world_pos(world: &World, entity: Entity) -> Vec3 {
        world
            .get_component::<Transform>(entity)
            .map(|t| t.translation)
            .unwrap_or_default()
    }

    fn snapshot_parents(world: &World) -> Vec<(Entity, Entity)> {
        world
            .all_entities()
            .filter_map(|e| {
                world
                    .get_component::<Parent>(e)
                    .map(|p| (e, p.0))
            })
            .collect()
    }
}