//! Texture streaming: small mips first, sharper ones when they are needed, memory kept in budget.
//!
//! A [`TextureStreamer`] owns the streaming state of any number of textures. Each frame:
//!
//! 1. [`TextureStreamer::update_from_scene`] works out, from the on-screen size of every object
//!    that uses a streamed texture, the sharpest mip worth having (one texel per pixel).
//! 2. [`TextureStreamer::update`] opens textures whose header arrived, uploads the mips that
//!    finished reading (within a byte budget), promotes a texture when every mip it is missing
//!    has arrived, and demotes textures that are no longer needed or do not fit the memory
//!    budget.
//!
//! A texture on the GPU holds only the mips `base..` (`base` = 0 is full size), so memory
//! follows what is resident. Promotion and demotion rebuild the texture with a GPU copy of the
//! levels both versions share; the old texture keeps rendering until the new one is complete.
//! Materials that use the texture get their bind group rebuilt.

use std::collections::{BTreeMap, HashMap, HashSet};

use redox_asset::{
    AssetManager, AssetStatus, Handle, Ktx2Texture, Priority, StreamInfo, StreamedTexture,
};
use redox_math::{Mat4, Vec3};

use crate::registry::TextureSlot;
use crate::renderer::Renderer;
use crate::resource::texture::Texture;
use crate::systems::RenderObject;

/// Settings of a [`TextureStreamer`].
#[derive(Clone, Copy, Debug)]
pub struct StreamConfig {
    /// Bytes of mip data uploaded to the GPU per [`TextureStreamer::update`] (at least one mip
    /// is always uploaded so a big level cannot starve).
    pub upload_budget_bytes: usize,
    /// Resident texture memory the streamer stays under by demoting the least recently used.
    pub texture_budget_bytes: usize,
    /// Longest edge, in pixels, of the mip every texture keeps resident from the start.
    pub floor_edge: u32,
    /// A texture keeps a sharp mip this many updates after it was last needed before it is
    /// demoted (stops flicker when an object moves in and out of view).
    pub demote_after_frames: u64,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            upload_budget_bytes: 8 << 20,
            texture_budget_bytes: 512 << 20,
            floor_edge: 64,
            demote_after_frames: 120,
        }
    }
}

/// Counters for a debug overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// Streamed textures (ready or still opening).
    pub textures: usize,
    /// Textures whose header has been read.
    pub ready: usize,
    /// Bytes of resident mips.
    pub resident_bytes: usize,
    /// Mip reads in flight or waiting for their promotion.
    pub pending_mips: usize,
    /// Promotions and demotions performed so far.
    pub rebuilds: u64,
}

const NEVER: u64 = u64::MAX;

struct State {
    handle: Handle<StreamedTexture>,
    info: Option<StreamInfo>,
    slot: Option<usize>,
    /// Sharpest mip on the GPU.
    base: u32,
    /// The mip chain from here down is always resident.
    floor: u32,
    /// Sharpest mip asked for this frame, and the frame that asked.
    desired: u32,
    desired_frame: u64,
    /// Last frame in which the resident base was actually needed.
    last_needed: u64,
    /// Reads started and not yet received.
    inflight: HashSet<u32>,
    /// Received mips sharper than `base`, waiting for the rest of their promotion.
    pending: BTreeMap<u32, Vec<u8>>,
    /// Which floor mips have been uploaded.
    floor_loaded: HashSet<u32>,
    failed: bool,
}

impl State {
    fn new(handle: Handle<StreamedTexture>) -> Self {
        Self {
            handle,
            info: None,
            slot: None,
            base: 0,
            floor: 0,
            desired: 0,
            desired_frame: NEVER,
            last_needed: 0,
            inflight: HashSet::new(),
            pending: BTreeMap::new(),
            floor_loaded: HashSet::new(),
            failed: false,
        }
    }

    fn ready(&self) -> Option<(StreamInfo, usize)> {
        Some((self.info?, self.slot?))
    }

    fn want(&mut self, mip: u32, frame: u64) {
        if self.desired_frame == frame {
            self.desired = self.desired.min(mip);
        } else {
            self.desired = mip;
            self.desired_frame = frame;
        }
    }
}

/// Streams textures into a [`Renderer`]. See the module documentation.
pub struct TextureStreamer {
    config: StreamConfig,
    states: Vec<State>,
    by_slot: HashMap<usize, usize>,
    frame: u64,
    rebuilds: u64,
}

/// A texture description with the mips `base..` and no data (for allocation).
fn shell(info: &StreamInfo, base: u32) -> Ktx2Texture {
    let (width, height) = info.mip_size(base);
    Ktx2Texture {
        width,
        height,
        format: info.format,
        layers: info.layers,
        faces: info.faces,
        levels: vec![Vec::new(); (info.mips - base) as usize],
    }
}

/// Whether mip `mip` can be the top level of a GPU texture (block formats need whole blocks).
fn can_be_base(info: &StreamInfo, mip: u32) -> bool {
    if !info.format.is_block_compressed() {
        return true;
    }
    let (w, h) = info.mip_size(mip);
    w.is_multiple_of(4) && h.is_multiple_of(4)
}

/// The smallest mip index `>= mip` (up to `limit`) that can be a base.
fn usable_base(info: &StreamInfo, mip: u32, limit: u32) -> u32 {
    let mut m = mip.min(limit);
    while m < limit && !can_be_base(info, m) {
        m += 1;
    }
    m
}

/// The mip kept resident: the sharpest usable one that is at most `edge` pixels long (or the
/// smallest usable one when none is that small).
fn floor_mip(info: &StreamInfo, edge: u32) -> u32 {
    let usable = |mip: &u32| can_be_base(info, *mip);
    let small_enough = |mip: &u32| {
        let (w, h) = info.mip_size(*mip);
        w.max(h) <= edge
    };
    (0..info.mips)
        .filter(usable)
        .find(small_enough)
        .or_else(|| (0..info.mips).rev().find(usable))
        .unwrap_or(info.mips - 1)
}

/// Writes mip `mip` (all layers and faces) into `storage`, whose top level is mip `base`.
fn upload_mip(
    queue: &wgpu::Queue,
    storage: &wgpu::Texture,
    info: &StreamInfo,
    base: u32,
    mip: u32,
    data: Vec<u8>,
) {
    let mut ktx = shell(info, base);
    let index = (mip.wrapping_sub(base)) as usize;
    if mip < base || index >= ktx.levels.len() || data.len() != info.mip_bytes(mip) {
        return;
    }
    ktx.levels[index] = data;
    Texture::upload_ktx2_level(queue, storage, &ktx, index);
}

impl TextureStreamer {
    /// A streamer with the given settings.
    pub fn new(config: StreamConfig) -> Self {
        Self {
            config,
            states: Vec::new(),
            by_slot: HashMap::new(),
            frame: 0,
            rebuilds: 0,
        }
    }

    /// Current settings.
    pub fn config(&self) -> StreamConfig {
        self.config
    }

    /// Changes the memory budget (textures are demoted on the next update if needed).
    pub fn set_texture_budget(&mut self, bytes: usize) {
        self.config.texture_budget_bytes = bytes;
    }

    /// Starts streaming the texture at `path`. Once [`slot_of`](Self::slot_of) is `Some`, a
    /// material can refer to it with `Handle::<Ktx2Texture>::new(handle.id())` in
    /// `MaterialData::with_albedo_ktx2` and friends.
    pub fn add(
        &mut self,
        assets: &mut AssetManager,
        path: &str,
        priority: Priority,
    ) -> Result<Handle<StreamedTexture>, redox_asset::AssetError> {
        let handle = assets.stream_texture(path, priority)?;
        if !self.states.iter().any(|s| s.handle == handle) {
            self.states.push(State::new(handle));
        }
        Ok(handle)
    }

    /// The registry slot of a streamed texture once its header has been read and the texture
    /// created.
    pub fn slot_of(&self, handle: Handle<StreamedTexture>) -> Option<usize> {
        self.state(handle).and_then(|s| s.slot)
    }

    /// The sharpest mip currently on the GPU (`None` until the texture exists).
    pub fn resident_base(&self, handle: Handle<StreamedTexture>) -> Option<u32> {
        self.state(handle)
            .filter(|s| s.slot.is_some())
            .map(|s| s.base)
    }

    fn state(&self, handle: Handle<StreamedTexture>) -> Option<&State> {
        self.states.iter().find(|s| s.handle == handle)
    }

    /// Says that `handle` should have mip `mip` (0 = full size) or sharper this frame. Without
    /// any call a texture is kept at full size.
    pub fn set_desired_mip(&mut self, handle: Handle<StreamedTexture>, mip: u32) {
        let frame = self.frame;
        if let Some(state) = self.states.iter_mut().find(|s| s.handle == handle) {
            state.want(mip, frame);
        }
    }

    /// Counters for a debug overlay.
    pub fn stats(&self) -> StreamStats {
        StreamStats {
            textures: self.states.len(),
            ready: self.states.iter().filter(|s| s.slot.is_some()).count(),
            resident_bytes: self
                .states
                .iter()
                .filter_map(|s| s.ready().map(|(info, _)| info.bytes_from(s.base)))
                .sum(),
            pending_mips: self
                .states
                .iter()
                .map(|s| s.inflight.len() + s.pending.len())
                .sum(),
            rebuilds: self.rebuilds,
        }
    }

    /// Works out the mip each streamed texture needs from how big its users are on screen
    /// (one texel per pixel). Call once per frame, after the camera is set and before
    /// [`update`](Self::update).
    pub fn update_from_scene(&mut self, renderer: &Renderer, objects: &[RenderObject]) {
        let camera = &renderer.camera_uniform;
        let view_proj = Mat4::from_cols_array_2d(&camera.view_proj);
        let eye = Vec3::new(
            camera.camera_pos[0],
            camera.camera_pos[1],
            camera.camera_pos[2],
        );
        let proj_scale = camera.proj[1][1].abs().max(1e-4); // 1 / tan(fov / 2)
        let viewport_h = renderer.internal_height.max(1) as f32;

        let mut wanted: HashMap<usize, u32> = HashMap::new();
        for object in objects {
            let center = object.bounds.center();
            let radius = object.bounds.half_extents().length().max(1e-4);
            if (view_proj * center.extend(1.0)).w + radius < 0.0 {
                continue; // behind the camera
            }
            let distance = ((center - eye).length() - radius).max(0.05);
            // Height of the object's bounding sphere on screen, in pixels.
            let screen_px = (radius * proj_scale / distance * viewport_h).max(1.0);
            let Some(material) = renderer.registry.legacy_material(object.material_index) else {
                continue;
            };
            for slot in [
                material.texture_index,
                material.normal_texture_index,
                material.mr_texture_index,
            ]
            .into_iter()
            .flatten()
            {
                let Some(&state) = self.by_slot.get(&slot) else {
                    continue;
                };
                let Some((info, _)) = self.states[state].ready() else {
                    continue;
                };
                let texels_per_pixel = info.height as f32 / screen_px;
                let mip = (texels_per_pixel.max(1.0).log2().floor() as u32).min(info.mips - 1);
                let entry = wanted.entry(state).or_insert(u32::MAX);
                *entry = (*entry).min(mip);
            }
        }
        let frame = self.frame;
        for (state, mip) in wanted {
            self.states[state].want(mip, frame);
        }
    }

    /// One streaming step: open textures, receive and upload mips, promote, demote, request.
    pub fn update(&mut self, renderer: &mut Renderer, assets: &mut AssetManager) {
        self.open_ready_textures(renderer, assets);
        self.receive_mips(renderer, assets);
        let targets = self.plan_targets();
        for (index, target) in targets.into_iter().enumerate() {
            self.apply_target(renderer, assets, index, target);
        }
        self.frame += 1;
    }

    fn open_ready_textures(&mut self, renderer: &mut Renderer, assets: &AssetManager) {
        for index in 0..self.states.len() {
            let state = &mut self.states[index];
            if state.info.is_some() || state.failed {
                continue;
            }
            match assets.stream_status(state.handle) {
                AssetStatus::Failed => state.failed = true,
                AssetStatus::Loading => {}
                AssetStatus::Ready => {
                    let Some(info) = assets.stream_info(state.handle) else {
                        continue;
                    };
                    let floor = floor_mip(&info, self.config.floor_edge);
                    let ktx = shell(&info, floor);
                    if !Texture::supports_ktx2(&renderer.device, &ktx) {
                        log::warn!("texture stream: format {:?} is not supported", info.format);
                        state.failed = true;
                        continue;
                    }
                    let storage =
                        Texture::create_ktx2_storage(&renderer.device, &ktx, "streamed_texture");
                    let texture = Texture::from_ktx2_storage(
                        &renderer.device,
                        storage,
                        &ktx,
                        "streamed_texture",
                    );
                    let bind_group = renderer
                        .shared_layouts
                        .create_texture_bind_group(&renderer.device, &texture);
                    let slot = renderer.registry.insert_texture(
                        state.handle.id(),
                        TextureSlot {
                            texture,
                            bind_group: Some(bind_group),
                        },
                    );
                    state.info = Some(info);
                    state.slot = Some(slot);
                    state.base = floor;
                    state.floor = floor;
                    self.by_slot.insert(slot, index);
                }
            }
        }
    }

    /// Takes finished mip reads: mips inside the resident range go straight to the GPU, sharper
    /// ones wait for the rest of their promotion.
    fn receive_mips(&mut self, renderer: &mut Renderer, assets: &mut AssetManager) {
        for ready in assets.drain_mips(self.config.upload_budget_bytes) {
            let Some(state) = self
                .states
                .iter_mut()
                .find(|s| s.handle.id() == ready.texture)
            else {
                continue;
            };
            state.inflight.remove(&ready.mip);
            let Ok(data) = ready.data else {
                log::warn!("texture stream: mip {} could not be read", ready.mip);
                state.failed = true;
                continue;
            };
            let Some((info, _)) = state.ready() else {
                continue;
            };
            if ready.mip < state.base {
                state.pending.insert(ready.mip, data);
                continue;
            }
            if let Some(slot) = renderer.registry.texture_slot_mut(state.handle.id()) {
                upload_mip(
                    &renderer.queue,
                    &slot.texture.texture,
                    &info,
                    state.base,
                    ready.mip,
                    data,
                );
            }
            if ready.mip >= state.floor {
                state.floor_loaded.insert(ready.mip);
            }
        }
    }

    /// The base mip each texture should have after this frame: asked-for sharpness wins at
    /// once, giving it up waits out the hysteresis, and the memory budget demotes the least
    /// recently needed textures.
    fn plan_targets(&mut self) -> Vec<u32> {
        let frame = self.frame;
        let hold = self.config.demote_after_frames;
        let mut targets: Vec<u32> = self
            .states
            .iter_mut()
            .map(|s| {
                let Some((info, _)) = s.ready() else {
                    return s.base;
                };
                let wanted = match s.desired_frame {
                    NEVER => 0,
                    f if f == frame => s.desired,
                    _ => s.floor,
                };
                let wanted = usable_base(&info, wanted, s.floor);
                if wanted <= s.base {
                    s.last_needed = frame;
                    wanted
                } else if frame.saturating_sub(s.last_needed) > hold {
                    wanted
                } else {
                    s.base
                }
            })
            .collect();

        let live: Vec<usize> = (0..targets.len())
            .filter(|&i| self.states[i].slot.is_some())
            .collect();
        let bytes = |i: usize, mip: u32| self.states[i].info.map_or(0, |info| info.bytes_from(mip));
        let mut total: usize = live.iter().map(|&i| bytes(i, targets[i])).sum();
        let mut order = live;
        order.sort_by_key(|&i| self.states[i].last_needed);
        'evict: while total > self.config.texture_budget_bytes {
            for &i in &order {
                let state = &self.states[i];
                let Some(info) = state.info else { continue };
                if targets[i] >= state.floor {
                    continue;
                }
                let before = bytes(i, targets[i]);
                targets[i] = usable_base(&info, targets[i] + 1, state.floor);
                total = total.saturating_sub(before - bytes(i, targets[i]));
                continue 'evict;
            }
            break; // everything is already at its floor
        }
        targets
    }

    fn apply_target(
        &mut self,
        renderer: &mut Renderer,
        assets: &mut AssetManager,
        index: usize,
        target: u32,
    ) {
        let state = &mut self.states[index];
        let Some((info, slot)) = state.ready() else {
            return;
        };
        if state.failed {
            return;
        }
        // The resident floor mips come first so a texture is never left blank.
        for mip in state.floor..info.mips {
            if !state.floor_loaded.contains(&mip)
                && !state.inflight.contains(&mip)
                && assets.request_mip(state.handle, mip, Priority::URGENT)
            {
                state.inflight.insert(mip);
            }
        }
        if target < state.base {
            for mip in target..state.base {
                if state.pending.contains_key(&mip) || state.inflight.contains(&mip) {
                    continue;
                }
                let priority = if mip == target {
                    Priority::HIGH
                } else {
                    Priority::NORMAL
                };
                if assets.request_mip(state.handle, mip, priority) {
                    state.inflight.insert(mip);
                }
            }
            if (target..state.base).all(|m| state.pending.contains_key(&m)) {
                rebuild(renderer, state, &info, target);
                renderer.refresh_materials_using_texture(slot);
                self.rebuilds += 1;
            }
        } else if target > state.base {
            rebuild(renderer, state, &info, target);
            renderer.refresh_materials_using_texture(slot);
            self.rebuilds += 1;
        }
    }
}

/// Replaces the GPU texture of `state` by one whose top level is mip `new_base`: the levels both
/// share are copied on the GPU, received mips fill the new sharper ones.
fn rebuild(renderer: &mut Renderer, state: &mut State, info: &StreamInfo, new_base: u32) {
    let old_base = state.base;
    let ktx = shell(info, new_base);
    let storage = Texture::create_ktx2_storage(&renderer.device, &ktx, "streamed_texture");
    let Some(slot) = renderer.registry.texture_slot_mut(state.handle.id()) else {
        return;
    };
    let block = if info.format.is_block_compressed() {
        4
    } else {
        1
    };
    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("texture_stream_rebuild"),
        });
    for mip in old_base.max(new_base)..info.mips {
        let (w, h) = info.mip_size(mip);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &slot.texture.texture,
                mip_level: mip - old_base,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &storage,
                mip_level: mip - new_base,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: w.next_multiple_of(block),
                height: h.next_multiple_of(block),
                depth_or_array_layers: info.layers * info.faces,
            },
        );
    }
    renderer.queue.submit(Some(encoder.finish()));
    for mip in new_base..old_base {
        if let Some(data) = state.pending.remove(&mip) {
            upload_mip(&renderer.queue, &storage, info, new_base, mip, data);
        }
    }
    let texture = Texture::from_ktx2_storage(&renderer.device, storage, &ktx, "streamed_texture");
    let bind_group = renderer
        .shared_layouts
        .create_texture_bind_group(&renderer.device, &texture);
    if let Some(slot) = renderer.registry.texture_slot_mut(state.handle.id()) {
        slot.texture = texture;
        slot.bind_group = Some(bind_group);
    }
    state.base = new_base;
    // Data for mips that are now resident is not needed any more.
    state.pending.retain(|&m, _| m < new_base);
}
