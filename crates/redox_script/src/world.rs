//! The game world as scripts see it (host API v1b and v2).

/// Identifier of an entity as scripts see it: an opaque number, never an ECS handle.
///
/// The host maps the number to one of its entities. An id is only meaningful for the world that
/// returned it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScriptEntityId(pub u32);

/// Queries a script may make into the game world. Implemented by the host, for example on top of
/// the physics context, and owned by the script instance it is installed on
/// ([`ScriptRuntime::set_world_queries`](crate::ScriptRuntime::set_world_queries)).
///
/// Every method except [`raycast`](Self::raycast) has a default that answers "nothing", so a host
/// written against the v1 API keeps working unchanged.
pub trait WorldQueries {
    /// Distance to the first solid surface along the ray, or `None` when nothing is hit within
    /// `max_distance` metres. `direction` is a unit vector.
    fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<f32>;

    /// The first entity hit by the ray and the distance to it; `None` on a miss.
    fn raycast_entity(
        &mut self,
        _origin: [f32; 3],
        _direction: [f32; 3],
        _max_distance: f32,
    ) -> Option<(ScriptEntityId, f32)> {
        None
    }

    /// Appends the entities whose volume overlaps the sphere to `out`, at most `limit` of them.
    fn overlap_sphere(
        &mut self,
        _center: [f32; 3],
        _radius: f32,
        _out: &mut Vec<ScriptEntityId>,
        _limit: usize,
    ) {
    }

    /// Appends the entities whose volume overlaps the axis aligned box to `out`, at most `limit`.
    fn overlap_box(
        &mut self,
        _center: [f32; 3],
        _half_extent: [f32; 3],
        _out: &mut Vec<ScriptEntityId>,
        _limit: usize,
    ) {
    }

    /// The entity with the given name, if one exists.
    fn find_by_name(&mut self, _name: &str) -> Option<ScriptEntityId> {
        None
    }

    /// Appends the entities carrying the tag to `out`, at most `limit` of them.
    fn find_by_tag(&mut self, _tag: &str, _out: &mut Vec<ScriptEntityId>, _limit: usize) {}

    /// World position of the entity, or `None` when it does not exist.
    fn position_of(&mut self, _id: ScriptEntityId) -> Option<[f32; 3]> {
        None
    }

    /// Named numeric value of another entity, or `None` when it has no such value.
    fn get_value(&mut self, _id: ScriptEntityId, _key: &str) -> Option<f32> {
        None
    }

    /// Appends a walkable polyline from `from` to `to` to `out` (at most `limit` points). Returns
    /// `false` when there is no path; `out` is then left empty.
    fn nav_path(
        &mut self,
        _from: [f32; 3],
        _to: [f32; 3],
        _out: &mut Vec<[f32; 3]>,
        _limit: usize,
    ) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host written against the v1 API: it only knows raycasts.
    struct OldHost;

    impl WorldQueries for OldHost {
        fn raycast(&mut self, _: [f32; 3], _: [f32; 3], _: f32) -> Option<f32> {
            Some(1.0)
        }
    }

    #[test]
    fn default_methods_return_empty_so_old_hosts_still_work() {
        let mut host = OldHost;
        let origin = [0.0; 3];
        assert_eq!(host.raycast(origin, [0.0, -1.0, 0.0], 5.0), Some(1.0));
        assert_eq!(host.raycast_entity(origin, [0.0, -1.0, 0.0], 5.0), None);
        let mut ids = Vec::new();
        host.overlap_sphere(origin, 2.0, &mut ids, 8);
        host.overlap_box(origin, [1.0; 3], &mut ids, 8);
        host.find_by_tag("enemy", &mut ids, 8);
        assert!(ids.is_empty());
        assert_eq!(host.find_by_name("player"), None);
        assert_eq!(host.position_of(ScriptEntityId(1)), None);
        assert_eq!(host.get_value(ScriptEntityId(1), "hp"), None);
        let mut path = Vec::new();
        assert!(!host.nav_path(origin, [4.0; 3], &mut path, 32));
        assert!(path.is_empty());
    }

    #[test]
    fn script_entity_ids_are_plain_numbers() {
        let id = ScriptEntityId(7);
        assert_eq!(id.0, 7);
        assert_eq!(std::mem::size_of::<ScriptEntityId>(), 4);
        assert_eq!(ScriptEntityId(7), id);
    }
}
