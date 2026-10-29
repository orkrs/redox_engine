//! Off-mesh links: jumps, ladders and doors that connect two places of the mesh.

use std::collections::HashSet;

use landmass::Vec3 as LmVec3;
use redox_math::Vec3;

use crate::backend::{self, LinkHandle};
use crate::error::NavError;
use crate::world::{AgentClass, NavWorld};

/// Largest distance between a link end and the navigation mesh, in metres.
pub const LINK_MAX_OFF_MESH: f32 = 0.5;
/// Half the length of the edge a point link is turned into, in metres.
const LINK_EDGE_HALF_LENGTH: f32 = 0.25;

/// Handle of a link added with [`NavWorld::add_link`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NavLinkId(pub(crate) u32);

/// A link between two points. Both ends must be within [`LINK_MAX_OFF_MESH`] of the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavLink {
    /// Where the link starts.
    pub from: Vec3,
    /// Where the link ends.
    pub to: Vec3,
    /// Whether agents may use the link from `to` to `from` as well.
    pub bidirectional: bool,
    /// Bit `n` allows agent class `n` to use the link.
    pub class_mask: u32,
    /// Cost of using the link, as a multiplier of its length (positive, finite).
    pub cost: f32,
}

/// Backend state of one link.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LinkEntry {
    pub(crate) handle: LinkHandle,
    pub(crate) class_mask: u32,
}

impl NavWorld {
    /// Adds a link. Its kind is its class mask, which is how agent classes filter links.
    ///
    /// # Errors
    ///
    /// [`NavError::BadCost`] for a cost that is not positive and finite,
    /// [`NavError::LinkOffMesh`] when an end is more than [`LINK_MAX_OFF_MESH`] from the mesh.
    pub fn add_link(&mut self, link: NavLink) -> Result<NavLinkId, NavError> {
        if !(link.cost.is_finite() && link.cost > 0.0) {
            return Err(NavError::BadCost);
        }
        let on_from = self
            .nearest_point(link.from, LINK_MAX_OFF_MESH)
            .ok_or(NavError::LinkOffMesh)?;
        let on_to = self
            .nearest_point(link.to, LINK_MAX_OFF_MESH)
            .ok_or(NavError::LinkOffMesh)?;
        let handle = self.archipelago.add_animation_link(backend::link(
            edge_at(on_from),
            edge_at(on_to),
            link.class_mask as usize,
            link.cost,
            link.bidirectional,
        ));
        let id = NavLinkId(self.next_link);
        self.next_link = self
            .next_link
            .checked_add(1)
            .ok_or_else(|| NavError::Backend("link ids are exhausted".into()))?;
        self.links.insert(
            id,
            LinkEntry {
                handle,
                class_mask: link.class_mask,
            },
        );
        self.refresh();
        Ok(id)
    }

    /// Removes a link; returns whether it existed.
    pub fn remove_link(&mut self, id: NavLinkId) -> bool {
        let Some(entry) = self.links.remove(&id) else {
            return false;
        };
        if self.archipelago.get_animation_link(entry.handle).is_some() {
            self.archipelago.remove_animation_link(entry.handle);
        }
        self.refresh();
        true
    }

    /// Link kinds (class masks) that agents of `class` may use.
    pub(crate) fn permitted_kinds(&self, class: AgentClass) -> HashSet<usize> {
        let bit = 1u32.checked_shl(u32::from(class.0)).unwrap_or(0);
        self.links
            .values()
            .filter(|entry| entry.class_mask & bit != 0)
            .map(|entry| entry.class_mask as usize)
            .collect()
    }
}

/// A short edge centred on a point, so the link attaches to the polygon under it.
fn edge_at(point: Vec3) -> (LmVec3, LmVec3) {
    let centre = backend::to_lm(point);
    let half = LmVec3::new(LINK_EDGE_HALF_LENGTH, 0.0, 0.0);
    (centre - half, centre + half)
}
