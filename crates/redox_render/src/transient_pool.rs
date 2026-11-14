//! Physical GPU textures for transient frame-graph slots (RND-12 step 2).
//!
//! One [`wgpu::Texture`] is allocated per compiled slot; logical graph textures with disjoint
//! lifetimes and equal [`TextureDesc`] share the same physical texture.

use std::collections::HashMap;

use crate::frame_graph::FrameFeatures;
use crate::memory_budget::texture_size_bytes;
use crate::render_graph::{CompiledGraph, TextureDesc};

/// One physical slot and the logical texture names that alias it this frame.
#[derive(Debug)]
pub struct PoolSlot {
    /// Descriptor shared by every logical texture in the slot.
    pub desc: TextureDesc,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Logical textures assigned to this slot by the compiler.
    pub users: Vec<&'static str>,
}

/// Cache key for pool rebuilds (resolution + enabled frame features).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub width: u32,
    pub height: u32,
    pub features: FrameFeatures,
}

/// Owns slot textures for the current compiled frame graph.
#[derive(Debug, Default)]
pub struct TransientPool {
    slots: Vec<PoolSlot>,
    /// Logical texture name → slot index.
    logical: HashMap<&'static str, usize>,
    key: Option<PoolKey>,
    /// Times `create_texture` was called (for resize tests).
    textures_created: u64,
}

impl TransientPool {
    /// Empty pool (no GPU resources).
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of physical slot textures.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// How many GPU textures were created since the last `reset_creation_counter`.
    pub fn textures_created(&self) -> u64 {
        self.textures_created
    }

    /// Clears the creation counter (used by resize tests).
    pub fn reset_creation_counter(&mut self) {
        self.textures_created = 0;
    }

    /// Rebuilds slots for `graph`. Reuses unchanged slot descriptors when `key` matches.
    pub fn rebuild(
        &mut self,
        device: &wgpu::Device,
        graph: &CompiledGraph,
        key: PoolKey,
    ) {
        let plan = graph.slot_plan();
        let same_key = self.key == Some(key);
        let mut next_slots = Vec::with_capacity(plan.len());
        for (slot_idx, entry) in plan.iter().enumerate() {
            let reuse = same_key
                .then(|| self.slots.get(slot_idx))
                .filter(|old| old.desc == entry.desc);
            let (texture, view) = if let Some(old) = reuse {
                (old.texture.clone(), old.view.clone())
            } else {
                self.textures_created += 1;
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("transient_slot"),
                    size: wgpu::Extent3d {
                        width: entry.desc.width.max(1),
                        height: entry.desc.height.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: entry.desc.mip_level_count.max(1),
                    sample_count: entry.desc.sample_count.max(1),
                    dimension: wgpu::TextureDimension::D2,
                    format: entry.desc.format,
                    usage: entry.desc.usage,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            };
            next_slots.push(PoolSlot {
                desc: entry.desc,
                texture,
                view,
                users: entry.users.clone(),
            });
        }
        self.slots = next_slots;
        self.logical.clear();
        for (slot_idx, entry) in plan.iter().enumerate() {
            for name in &entry.users {
                self.logical.insert(name, slot_idx);
            }
        }
        self.key = Some(key);
    }

    /// View of a logical transient texture, if the pool owns it.
    pub fn view(&self, name: &str) -> Option<&wgpu::TextureView> {
        self.logical
            .get(name)
            .and_then(|&i| self.slots.get(i))
            .map(|s| &s.view)
    }

    /// Texture of a logical transient texture, if the pool owns it.
    pub fn texture(&self, name: &str) -> Option<&wgpu::Texture> {
        self.logical
            .get(name)
            .and_then(|&i| self.slots.get(i))
            .map(|s| &s.texture)
    }

    /// Bytes saved by aliasing: sum of logical transient sizes minus sum of slot sizes.
    pub fn aliased_saved_bytes(&self, graph: &CompiledGraph) -> u64 {
        let logical = graph.logical_transient_bytes();
        let physical = self
            .slots
            .iter()
            .map(|s| slot_bytes(&s.desc))
            .sum::<u64>();
        logical.saturating_sub(physical)
    }

    /// GPU bytes of all slot textures (for [`MemoryCategory::Transient`]).
    pub fn gpu_bytes(&self) -> u64 {
        self.slots.iter().map(|s| slot_bytes(&s.desc)).sum()
    }

    /// Names of logical textures in each slot (for reports).
    pub fn slot_users(&self) -> impl Iterator<Item = (&PoolSlot, &[&'static str])> {
        self.slots
            .iter()
            .map(|s| (s, s.users.as_slice()))
    }
}

fn slot_bytes(desc: &TextureDesc) -> u64 {
    let bpp = desc.format.block_copy_size(None).unwrap_or(4);
    texture_size_bytes(
        desc.width,
        desc.height,
        bpp,
        desc.mip_level_count.max(1),
    )
}