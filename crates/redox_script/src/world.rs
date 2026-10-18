//! The game world as scripts see it (host API v1b).

/// Queries a script may make into the game world. Implemented by the host, for example on top of
/// the physics context, and owned by the script instance it is installed on
/// ([`ScriptRuntime::set_world_queries`](crate::ScriptRuntime::set_world_queries)).
pub trait WorldQueries {
    /// Distance to the first solid surface along the ray, or `None` when nothing is hit within
    /// `max_distance` metres. `direction` is a unit vector.
    fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<f32>;
}
