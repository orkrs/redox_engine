//! Debug drawing of navigation data: polygon edges and planned paths, as line segments.

use redox_ecs::World;
use redox_math::Vec3;

use crate::components::NavPath;
use crate::world::NavWorld;

/// Resource with the line segments to draw this frame. The game draws `lines` with its line renderer.
#[derive(Debug, Default)]
pub struct NavDebugDraw {
    /// Draw the edges of every polygon.
    pub draw_polygons: bool,
    /// Draw the paths of agents.
    pub draw_paths: bool,
    /// Segments (start, end) for the current frame.
    pub lines: Vec<[Vec3; 2]>,
}

impl NavDebugDraw {
    /// Rebuilds `lines` from the world's mesh and the agents' paths, as the flags ask.
    pub fn fill(&mut self, nav: &NavWorld, world: &World) {
        self.lines.clear();
        if self.draw_polygons {
            self.lines.extend(nav.polygon_edges());
        }
        if self.draw_paths {
            for path in world.query::<&NavPath>().iter(world) {
                for pair in path.points.windows(2) {
                    self.lines.push([pair[0], pair[1]]);
                }
            }
        }
    }
}
