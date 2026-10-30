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
use redox_math::{GlobalTransform, Vec3};
use redox_render::RenderObject;
use redox_render::camera::ActiveCamera;
use redox_render::hlod::{CellLod, HlodConfig, opacities, step_cell_lod};
use redox_render::hlod_gpu::{ProxyGpu, proxy_render_object};
use redox_render::systems::FrameRenderObjects;
use redox_scene::{ActorKindTable, PartitionCell, StreamedActor};

use crate::time::Time;

/// Render-object ids of proxies start here, away from entity ids.
///
/// ASSUMPTION: entity indices stay below this value in practice, so proxy ids do
/// not collide with actor ids.
const PROXY_ID_BASE: u32 = 0x8000_0000;

/// Per-cell HLOD state, the detail opacity of the actors it hides, and the
/// uploaded proxies.
#[derive(Debug)]
pub struct HlodRuntime {
    config: HlodConfig,
    cell_size: f32,
    origin: Vec3,
    baked: HashSet<PartitionCell>,
    states: HashMap<PartitionCell, CellLod>,
    detail_opacity: HashMap<u32, f32>,
    proxy_opacity: HashMap<PartitionCell, f32>,
    proxies: HashMap<PartitionCell, (u32, ProxyGpu)>,
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
            proxies: HashMap::new(),
        }
    }

    /// Registers the uploaded proxy of a baked cell. A cell registered twice keeps
    /// its first render-object id and takes the new proxy.
    pub fn register_proxy(&mut self, cell: PartitionCell, gpu: ProxyGpu) {
        let next_id = PROXY_ID_BASE + self.proxies.len() as u32;
        let id = self.proxies.get(&cell).map_or(next_id, |(id, _)| *id);
        self.proxies.insert(cell, (id, gpu));
    }

    /// Whether a cell has an uploaded proxy.
    pub fn has_proxy(&self, cell: PartitionCell) -> bool {
        self.proxies.contains_key(&cell)
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
    pub fn update(&mut self, members: &HashMap<PartitionCell, Vec<u32>>, viewer: Vec3, dt: f32) {
        self.detail_opacity.clear();
        let baked: Vec<PartitionCell> = self.baked.iter().copied().collect();
        for cell in baked {
            // Only cells with an uploaded proxy fade; the others keep their detail objects.
            if !self.proxies.contains_key(&cell) {
                continue;
            }
            let (detail, _) = self.advance(cell, viewer, dt);
            for &id in members.get(&cell).into_iter().flatten() {
                self.detail_opacity.insert(id, detail);
            }
        }
    }
}

/// Whether the HLOD frame step runs. Inserted by [`crate::hlod_load::HlodPlugin`];
/// with `enabled: false` the frame is left unchanged even when proxies exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HlodSettings {
    /// Runs the HLOD step when true.
    pub enabled: bool,
}

/// World position of the entity of [`ActiveCamera`], if both exist.
fn active_camera_position(world: &World) -> Option<Vec3> {
    let camera = world.get_resource::<ActiveCamera>()?.0;
    let matrix = world.get_component::<GlobalTransform>(camera)?.matrix;
    Some(Vec3::new(matrix.w_axis.x, matrix.w_axis.y, matrix.w_axis.z))
}

/// Frame step of HLOD: runs [`hlod_frame`] on the frame's render objects
/// (`FrameRenderObjects`, filled by extraction earlier in the frame) for the
/// active camera.
///
/// Does nothing unless [`HlodSettings`] is enabled, an [`HlodRuntime`] with at
/// least one proxy exists, and an active camera and frame objects are present.
pub fn hlod_frame_system(world: &mut World) {
    if !world
        .get_resource::<HlodSettings>()
        .is_some_and(|s| s.enabled)
    {
        return;
    }
    if world
        .get_resource::<HlodRuntime>()
        .is_none_or(|runtime| runtime.proxies.is_empty())
    {
        return;
    }
    let Some(viewer) = active_camera_position(world) else {
        return;
    };
    let dt = world.get_resource::<Time>().map_or(0.0, |t| t.delta_time);
    let Some(frame) = world.get_resource_mut::<FrameRenderObjects>() else {
        return;
    };
    let mut objects = std::mem::take(&mut frame.0);
    hlod_frame(world, viewer, dt, &mut objects);
    if let Some(frame) = world.get_resource_mut::<FrameRenderObjects>() {
        frame.0 = objects;
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

/// Applies HLOD to the render objects of a frame, after extraction.
///
/// Objects of HLOD actors are dropped at detail opacity zero, otherwise get
/// `color[3]` set to the opacity; other objects are left as they are. Then each
/// uploaded proxy with a positive opacity is appended, ordered by its id.
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
    let mut proxies: Vec<(u32, f32, &ProxyGpu)> = runtime
        .proxies
        .iter()
        .filter_map(|(cell, (id, gpu))| {
            let opacity = runtime.proxy_opacity(*cell);
            (opacity > 0.0).then_some((*id, opacity, gpu))
        })
        .collect();
    proxies.sort_by_key(|(id, _, _)| *id);
    objects.extend(
        proxies
            .into_iter()
            .map(|(id, opacity, gpu)| proxy_render_object(gpu, id, opacity)),
    );
}

/// One HLOD step for a frame: advances the cells and applies the result to the
/// frame's render objects. The frame loop calls this once, after render extraction.
pub fn hlod_frame(world: &mut World, viewer_pos: Vec3, dt: f32, objects: &mut Vec<RenderObject>) {
    hlod_system(world, viewer_pos, dt);
    apply_hlod_to_render_objects(world, objects);
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
        rt.register_proxy(far, dummy_gpu());
        let members = HashMap::from([(far, vec![7_u32, 8])]);
        for _ in 0..10 {
            rt.update(&members, Vec3::ZERO, 0.1);
        }
        assert_eq!(rt.detail_opacity(7), Some(0.0));
        assert_eq!(rt.detail_opacity(8), Some(0.0));
        assert_eq!(rt.detail_opacity(9), None);
    }

    fn dummy_gpu() -> ProxyGpu {
        ProxyGpu {
            mesh_slot: 0,
            material_slot: 0,
            bounds: redox_math::Aabb::empty(),
        }
    }

    #[test]
    fn a_cell_without_a_proxy_keeps_its_detail_objects() {
        let mut rt = runtime();
        let far = PartitionCell { x: 20, y: 0 };
        let members = HashMap::from([(far, vec![7_u32])]);
        for _ in 0..10 {
            rt.update(&members, Vec3::ZERO, 0.1);
        }
        assert_eq!(rt.detail_opacity(7), None);
        assert_eq!(rt.state(far), CellLod::Detail);
    }

    /// World with a crate kind, an active camera at the origin and one far cell
    /// (20, 0) that has a proxy. Returns the world and the ids of its two actors.
    fn far_cell_world(enabled: bool, with_proxy: bool) -> (World, Vec<u32>) {
        let mut world = World::new();
        world.insert_resource(
            ActorKindTable::from_ron(
                r#"(kinds: { "crate": (mesh: "c.glb", color: (0.5, 0.5, 0.5)) })"#,
            )
            .unwrap(),
        );
        let far = PartitionCell { x: 20, y: 0 };
        let mut ids = Vec::new();
        for n in 0..2 {
            let entity = world.spawn_bundle((StreamedActor {
                cell: far,
                id: format!("c{n}"),
                kind: "crate".into(),
            },));
            ids.push(entity.id());
        }
        let camera = world.spawn_bundle((GlobalTransform::IDENTITY,));
        world.insert_resource(ActiveCamera(camera));
        world.insert_resource(Time::new(1.0 / 60.0));
        let mut runtime = HlodRuntime::new(HlodConfig::default(), 50.0, Vec3::ZERO, [far]);
        if with_proxy {
            runtime.register_proxy(far, dummy_gpu());
        }
        world.insert_resource(runtime);
        world.insert_resource(HlodSettings { enabled });
        let objects = ids.iter().map(|id| detail_object(*id)).collect();
        world.insert_resource(FrameRenderObjects(objects));
        (world, ids)
    }

    fn detail_object(id: u32) -> RenderObject {
        RenderObject {
            id,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: redox_math::Mat4::IDENTITY,
            color: [1.0; 4],
            mesh_index: 0,
            material_index: 0,
            bounds: redox_math::Aabb::empty(),
        }
    }

    fn frame_len(world: &World) -> usize {
        world.get_resource::<FrameRenderObjects>().unwrap().0.len()
    }

    #[test]
    fn the_frame_step_does_nothing_when_disabled() {
        let (mut world, _) = far_cell_world(false, true);
        hlod_frame_system(&mut world);
        assert_eq!(frame_len(&world), 2);
    }

    #[test]
    fn the_frame_step_does_nothing_without_a_proxy() {
        let (mut world, _) = far_cell_world(true, false);
        for _ in 0..10 {
            hlod_frame_system(&mut world);
        }
        assert_eq!(frame_len(&world), 2);
    }

    #[test]
    fn a_far_cell_is_replaced_by_its_proxy_in_the_frame() {
        let (mut world, ids) = far_cell_world(true, true);
        for _ in 0..10 {
            // The frame is extracted anew every tick; the system appends the proxy to it.
            world.insert_resource(FrameRenderObjects(
                ids.iter().map(|id| detail_object(*id)).collect(),
            ));
            world.get_resource_mut::<Time>().unwrap().delta_time = 0.1;
            hlod_frame_system(&mut world);
        }
        let objects = &world.get_resource::<FrameRenderObjects>().unwrap().0;
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].id, PROXY_ID_BASE);
        assert_eq!(objects[0].color[3], 1.0);
    }
}
