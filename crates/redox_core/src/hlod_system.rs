//! Runtime of HLOD: every baked cell cross-fades between its detail objects and
//! its proxy by distance (state machine in `redox_render::hlod`).
//!
//! [`hlod_system`] advances the cells and remembers the detail opacity of each
//! actor of an HLOD kind. [`apply_hlod_to_render_objects`] runs after render
//! extraction: it writes the opacity into `RenderObject::color[3]` (the
//! renderer dithers on alpha) and drops objects whose opacity is zero. Drawing
//! the proxy mesh itself is left to the caller; its opacity is available from
//! [`HlodRuntime::proxy_opacity`].

use std::collections::{HashMap, HashSet};

use redox_ecs::{Entity, World};
use redox_math::Vec3;
use redox_render::RenderObject;
use redox_render::hlod::{CellLod, HlodConfig, opacities, step_cell_lod};
use redox_scene::{ActorKindTable, PartitionCell, StreamedActor};

/// Per-cell HLOD state and the detail opacity of the actors it hides.
#[derive(Debug)]
pub struct HlodRuntime {
    config: HlodConfig,
    cell_size: f32,
    origin: Vec3,
    baked: HashSet<PartitionCell>,
    states: HashMap<PartitionCell, CellLod>,
    detail_opacity: HashMap<u32, f32>,
    proxy_opacity: HashMap<PartitionCell, f32>,
}

impl HlodRuntime {
    /// Runtime for a world grid (`cell_size`, grid `origin`) and the cells that
    /// have a `.hlod` proxy. Cells without a proxy are never faded.
    pub fn new(
        config: HlodConfig,
        cell_size: f32,
        origin: Vec3,
        baked: impl IntoIterator<Item = PartitionCell>,
    ) -> Self {
        Self {
            config,
            cell_size,
            origin,
            baked: baked.into_iter().collect(),
            states: HashMap::new(),
            detail_opacity: HashMap::new(),
            proxy_opacity: HashMap::new(),
        }
    }

    /// Current state of a cell (`Detail` until it has been advanced).
    pub fn state(&self, cell: PartitionCell) -> CellLod {
        self.states.get(&cell).copied().unwrap_or(CellLod::Detail)
    }

    /// Opacity of the proxy of a cell in `0..=1` (0 for cells without a proxy).
    pub fn proxy_opacity(&self, cell: PartitionCell) -> f32 {
        self.proxy_opacity.get(&cell).copied().unwrap_or(0.0)
    }

    /// Opacity of the detail objects of an entity, if the entity belongs to a
    /// baked cell and an HLOD kind.
    pub fn detail_opacity(&self, entity_id: u32) -> Option<f32> {
        self.detail_opacity.get(&entity_id).copied()
    }

    /// World position of the centre of `cell` on the ground plane.
    fn centre(&self, cell: PartitionCell) -> Vec3 {
        Vec3::new(
            self.origin.x + (cell.x as f32 + 0.5) * self.cell_size,
            self.origin.y,
            self.origin.z + (cell.y as f32 + 0.5) * self.cell_size,
        )
    }

    /// Advances one cell by `dt` seconds for a viewer at `viewer`, and returns
    /// its `(detail, proxy)` opacities. The distance is measured on the ground plane.
    pub fn advance(&mut self, cell: PartitionCell, viewer: Vec3, dt: f32) -> (f32, f32) {
        let centre = self.centre(cell);
        let distance = Vec3::new(centre.x - viewer.x, 0.0, centre.z - viewer.z).length();
        let next = step_cell_lod(self.state(cell), distance, dt, &self.config);
        self.states.insert(cell, next);
        let (detail, proxy) = opacities(next);
        self.proxy_opacity.insert(cell, proxy);
        (detail, proxy)
    }

    /// Advances every baked cell and refreshes the detail opacity of the actors
    /// listed in `members` (entity ids grouped by cell).
    pub fn update(
        &mut self,
        members: &HashMap<PartitionCell, Vec<u32>>,
        viewer: Vec3,
        dt: f32,
    ) {
        self.detail_opacity.clear();
        let baked: Vec<PartitionCell> = self.baked.iter().copied().collect();
        for cell in baked {
            let (detail, _) = self.advance(cell, viewer, dt);
            for &id in members.get(&cell).into_iter().flatten() {
                self.detail_opacity.insert(id, detail);
            }
        }
    }
}

/// Advances the HLOD cells of the world for `viewer_pos` by `dt` seconds.
///
/// Does nothing when no [`HlodRuntime`] or no [`ActorKindTable`] resource is
/// present. Only actors whose kind has `hlod: true` are hidden by a proxy.
pub fn hlod_system(world: &mut World, viewer_pos: Vec3, dt: f32) {
    let Some(table) = world.get_resource::<ActorKindTable>() else {
        return;
    };
    let mut members: HashMap<PartitionCell, Vec<u32>> = HashMap::new();
    for (entity, actor) in world.query::<(Entity, &StreamedActor)>().iter(world) {
        if table.is_hlod(&actor.kind) {
            members.entry(actor.cell).or_default().push(entity.id());
        }
    }
    let Some(runtime) = world.get_resource_mut::<HlodRuntime>() else {
        return;
    };
    runtime.update(&members, viewer_pos, dt);
}

/// Applies the detail opacity of the HLOD actors to extracted render objects:
/// objects at opacity zero are removed, others get `color[3]` set to the opacity.
/// Objects that are not HLOD actors are left as they are.
pub fn apply_hlod_to_render_objects(world: &World, objects: &mut Vec<RenderObject>) {
    let Some(runtime) = world.get_resource::<HlodRuntime>() else {
        return;
    };
    objects.retain_mut(|object| match runtime.detail_opacity(object.id) {
        Some(opacity) if opacity <= 0.0 => false,
        Some(opacity) => {
            object.color[3] = opacity;
            true
        }
        None => true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> HlodRuntime {
        HlodRuntime::new(
            HlodConfig::default(),
            50.0,
            Vec3::ZERO,
            [PartitionCell { x: 0, y: 0 }, PartitionCell { x: 20, y: 0 }],
        )
    }

    #[test]
    fn a_near_cell_stays_in_detail() {
        let mut rt = runtime();
        let near = PartitionCell { x: 0, y: 0 };
        for _ in 0..20 {
            rt.advance(near, Vec3::ZERO, 0.1);
        }
        assert_eq!(rt.state(near), CellLod::Detail);
        assert_eq!(rt.proxy_opacity(near), 0.0);
    }

    #[test]
    fn a_far_cell_fades_to_its_proxy() {
        let mut rt = runtime();
        let far = PartitionCell { x: 20, y: 0 };
        let mut last = (1.0, 0.0);
        for _ in 0..10 {
            last = rt.advance(far, Vec3::ZERO, 0.1);
        }
        assert_eq!(last, (0.0, 1.0));
        assert_eq!(rt.state(far), CellLod::Proxy);
    }

    #[test]
    fn a_cell_without_a_proxy_is_never_advanced() {
        let rt = runtime();
        let unbaked = PartitionCell { x: 5, y: 5 };
        assert_eq!(rt.proxy_opacity(unbaked), 0.0);
        assert_eq!(rt.state(unbaked), CellLod::Detail);
    }

    #[test]
    fn update_sets_the_detail_opacity_of_member_actors() {
        let mut rt = runtime();
        let far = PartitionCell { x: 20, y: 0 };
        let members = HashMap::from([(far, vec![7_u32, 8])]);
        for _ in 0..10 {
            rt.update(&members, Vec3::ZERO, 0.1);
        }
        assert_eq!(rt.detail_opacity(7), Some(0.0));
        assert_eq!(rt.detail_opacity(8), Some(0.0));
        assert_eq!(rt.detail_opacity(9), None);
    }
}
