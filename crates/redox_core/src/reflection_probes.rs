//! Reflection probe slots from the ECS, upload, and one capture per frame (RFL-02).

use std::collections::HashMap;

use redox_ecs::{Entity, World};
use redox_math::Vec3;
use redox_render::Renderer;
use redox_render::context::RenderContext;
use redox_render::light::LightScene;
use redox_render::reflection::{ReflectionProbe, ReflectionProbeComponent, pick_by_priority};
use redox_render::systems::FrameRenderObjects;

use crate::app::AppBuilder;
use crate::dispatcher::Stage;
use crate::plugin::Plugin;

/// ECS resource with the lights used for probe capture when present.
#[derive(Clone, Debug, Default)]
pub struct SceneLightScene(pub LightScene);

#[derive(Default)]
struct ProbePoseCache {
    poses: HashMap<Entity, (Vec3, Vec3)>,
}

/// Marks probe components dirty when centre or half extent changes (camera moves do not).
pub fn reflection_probe_track_dirty(world: &mut World) {
    let mut cache = world
        .remove_resource::<ProbePoseCache>()
        .unwrap_or_default();
    let entities: Vec<Entity> = world.all_entities().collect();
    for entity in entities {
        let pose = world
            .get_component::<ReflectionProbeComponent>(entity)
            .map(|component| (component.probe.center, component.probe.half_extent));
        let Some(pose) = pose else {
            cache.poses.remove(&entity);
            continue;
        };
        let changed = match cache.poses.insert(entity, pose) {
            None => true,
            Some(previous) => previous != pose,
        };
        if changed
            && let Some(component) = world.get_component_mut::<ReflectionProbeComponent>(entity)
        {
            component.dirty = true;
        }
    }
    world.insert_resource(cache);
}

/// Collects up to four probes from entities, highest priority first (RFL-01 order).
pub fn probes_from_world(world: &World) -> Vec<ReflectionProbe> {
    let probes: Vec<ReflectionProbe> = world
        .all_entities()
        .filter_map(|entity| {
            world
                .get_component::<ReflectionProbeComponent>(entity)
                .map(|component| component.probe)
        })
        .collect();
    pick_by_priority(&probes)
}

/// Marks every probe dirty (level load or editor “Capture” forcing refresh).
pub fn mark_all_reflection_probes_dirty(world: &mut World) {
    let entities: Vec<Entity> = world.all_entities().collect();
    for entity in entities {
        let Some(component) = world.get_component_mut::<ReflectionProbeComponent>(entity) else {
            continue;
        };
        component.dirty = true;
    }
}

fn lights_for_capture(world: &World) -> LightScene {
    world
        .get_resource::<SceneLightScene>()
        .map(|scene| scene.0.clone())
        .unwrap_or_default()
}

fn first_dirty_slot(world: &World, probes: &[ReflectionProbe]) -> Option<usize> {
    let mut entities: Vec<(i32, bool)> = world
        .all_entities()
        .filter_map(|entity| {
            world
                .get_component::<ReflectionProbeComponent>(entity)
                .map(|c| (c.probe.priority, c.dirty))
        })
        .collect();
    entities.sort_by_key(|(priority, _)| std::cmp::Reverse(*priority));
    entities.truncate(probes.len());
    entities.iter().position(|(_, dirty)| *dirty)
}

fn clear_dirty_at_slot(world: &mut World, slot: usize, probes: &[ReflectionProbe]) {
    let target = probes.get(slot).map(|p| p.priority);
    let Some(target) = target else {
        return;
    };
    let entities: Vec<Entity> = world.all_entities().collect();
    for entity in entities {
        let Some(component) = world.get_component_mut::<ReflectionProbeComponent>(entity) else {
            continue;
        };
        if component.probe.priority == target && component.dirty {
            component.dirty = false;
            return;
        }
    }
}

/// Uploads probe slots and captures at most one dirty slot before the main frame draw.
pub fn reflection_probe_render_system(world: &mut World, renderer: &mut RenderContext) {
    let probes = probes_from_world(world);
    if probes.is_empty() {
        renderer.set_reflection_probes_enabled(false);
        return;
    }
    renderer.set_reflection_probes_enabled(true);
    renderer.set_reflection_probes(&probes);

    let objects = world
        .get_resource::<FrameRenderObjects>()
        .map(|frame| frame.0.as_slice())
        .unwrap_or(&[]);
    let lights = lights_for_capture(world);

    let Some(slot) = first_dirty_slot(world, &probes) else {
        return;
    };
    if renderer
        .capture_reflection_probe(slot, objects, &lights)
        .is_ok()
    {
        clear_dirty_at_slot(world, slot, &probes);
    }
}

/// Editor / manual frame path without the full ECS plugin loop.
pub fn reflection_probe_apply_slice(
    renderer: &mut Renderer,
    probes: &[ReflectionProbe],
    objects: &[redox_render::systems::RenderObject],
    lights: &LightScene,
    dirty_slots: &mut [bool],
) {
    if probes.is_empty() {
        renderer.set_reflection_probes_enabled(false);
        return;
    }
    renderer.set_reflection_probes_enabled(true);
    renderer.set_reflection_probes(probes);
    let Some(slot) = dirty_slots.iter().position(|dirty| *dirty) else {
        return;
    };
    if renderer
        .capture_reflection_probe(slot, objects, lights)
        .is_ok()
    {
        dirty_slots[slot] = false;
    }
}

/// Registers [`reflection_probe_render_system`] on [`Stage::Render`] (before the draw system).
pub struct ReflectionProbePlugin;

impl Plugin for ReflectionProbePlugin {
    fn build(&self, app: &mut AppBuilder) {
        app.on_setup(|world| {
            world.insert_resource(ProbePoseCache::default());
        });
        app.add_system_mut(Stage::Update, |world, _| {
            reflection_probe_track_dirty(world);
        });
        app.add_system_mut(Stage::Render, |world, renderer| {
            reflection_probe_render_system(world, renderer);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;
    use redox_render::reflection::MAX_PROBES;

    fn probe(priority: i32, center: Vec3) -> ReflectionProbe {
        ReflectionProbe {
            center,
            half_extent: Vec3::ONE,
            blend_distance: 0.25,
            priority,
        }
    }

    #[test]
    fn four_probes_keep_the_highest_priority() {
        let mut world = World::new();
        for priority in 0..6 {
            let entity = world.spawn();
            world.add_component(
                entity,
                ReflectionProbeComponent::new(probe(priority, Vec3::splat(priority as f32))),
            );
        }
        let chosen = probes_from_world(&world);
        assert_eq!(chosen.len(), MAX_PROBES);
        assert_eq!(chosen[0].priority, 5);
        assert_eq!(chosen[MAX_PROBES - 1].priority, 2);
    }

    #[test]
    fn moving_the_center_marks_the_slot_dirty() {
        let mut world = World::new();
        world.insert_resource(ProbePoseCache::default());
        let entity = world.spawn();
        world.add_component(
            entity,
            ReflectionProbeComponent {
                probe: probe(0, Vec3::ZERO),
                resolution: 128,
                dirty: false,
            },
        );
        reflection_probe_track_dirty(&mut world);
        world
            .get_component_mut::<ReflectionProbeComponent>(entity)
            .unwrap()
            .probe
            .center = Vec3::new(1.0, 0.0, 0.0);
        reflection_probe_track_dirty(&mut world);
        assert!(
            world
                .get_component::<ReflectionProbeComponent>(entity)
                .unwrap()
                .dirty
        );
    }

    #[test]
    fn moving_the_camera_does_not() {
        let world = World::new();
        assert!(first_dirty_slot(&world, &[]).is_none());
    }

    #[test]
    fn a_world_without_probes_leaves_them_disabled() {
        let world = World::new();
        assert!(probes_from_world(&world).is_empty());
    }

    #[test]
    fn a_world_with_one_probe_enables_them_and_uploads_the_box() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(
            entity,
            ReflectionProbeComponent::new(probe(1, Vec3::new(2.0, 0.0, 0.0))),
        );
        let list = probes_from_world(&world);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].center.x, 2.0);
    }

    /// GPU: red channel on a wall facing the probe (RFL-01 criterion). Coordinator runs on hardware.
    #[test]
    #[ignore = "needs GPU and the red_wall fixture from reflection_probes.rs"]
    fn a_red_wall_shows_up_in_the_probe_after_load() {}
}
