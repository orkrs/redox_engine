//! Render graph compiler: declares passes with the textures and buffers they access, then
//! orders them, drops the ones nobody needs, checks the declared usage flags, computes how long
//! each transient texture lives and assigns physical slots so textures with disjoint lifetimes
//! share memory.
//!
//! This module is pure CPU logic (no GPU calls). The renderer takes the pass *order* from it
//! (see [`crate::frame_graph`]); passes still own their textures, so the aliasing plan is
//! informational. wgpu already inserts resource barriers, so the graph's value is explicit
//! dependencies, culling, a single source of truth for the pass order and uniform profiling
//! labels - not barriers. The [`CompiledGraph::barrier_plan`] lists the access changes that wgpu
//! will perform, for reports and tests.
//!
//! # Semantics
//! * Passes are declared in a sensible sequential order. A pass that reads a resource depends
//!   on the **last earlier pass that wrote it**; a pass that writes a resource depends on
//!   earlier readers and writers of it (so declaration order resolves write-after-read).
//! * Reading a transient resource nobody wrote earlier is an error
//!   ([`GraphError::ReadBeforeWrite`]); imported (external) resources may be read freely.
//! * A pass that both reads and writes the same resource must say so with
//!   [`PassBuilder::read_write`]; otherwise it is [`GraphError::SelfDependency`].
//! * Every access must be covered by the usage flags the resource was declared with
//!   ([`GraphError::MissingUsage`] / [`GraphError::MissingBufferUsage`]).
//! * A pass is kept if it has side effects, writes an imported resource or writes something a
//!   kept pass reads. Everything else is culled.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use wgpu::{BufferUsages, TextureUsages};

/// Size, format and usage of a texture; two transient textures can share a physical slot
/// only when their descriptors are equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Texel format.
    pub format: wgpu::TextureFormat,
    /// Usages the texture needs.
    pub usage: TextureUsages,
}

/// Size and usage of a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferDesc {
    /// Size in bytes.
    pub size: u64,
    /// Usages the buffer needs.
    pub usage: BufferUsages,
}

/// Handle of a texture declared in a [`RenderGraph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TexHandle(u32);

/// Handle of a buffer declared in a [`RenderGraph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BufHandle(u32);

/// Index of a pass in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PassId(pub usize);

/// How a pass accesses a resource. The access decides which usage flag the resource must
/// declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    /// Rendered into as a colour attachment.
    ColorTarget,
    /// Rendered into as a depth attachment.
    DepthTarget,
    /// Sampled (textures) or bound read-only (buffers).
    ShaderRead,
    /// Read through a storage binding.
    StorageRead,
    /// Written through a storage binding.
    StorageWrite,
    /// Source of a copy.
    CopySrc,
    /// Destination of a copy.
    CopyDst,
}

impl Access {
    /// Texture usage flag this access requires.
    fn texture_usage(self) -> TextureUsages {
        match self {
            Access::ColorTarget | Access::DepthTarget => TextureUsages::RENDER_ATTACHMENT,
            Access::ShaderRead => TextureUsages::TEXTURE_BINDING,
            Access::StorageRead | Access::StorageWrite => TextureUsages::STORAGE_BINDING,
            Access::CopySrc => TextureUsages::COPY_SRC,
            Access::CopyDst => TextureUsages::COPY_DST,
        }
    }

    /// Buffer usage flag this access requires, or `None` when buffers cannot be accessed this
    /// way (attachments).
    // ASSUMPTION: a read-only buffer binding (`ShaderRead`) is a read-only storage binding.
    fn buffer_usage(self) -> Option<BufferUsages> {
        match self {
            Access::ColorTarget | Access::DepthTarget => None,
            Access::ShaderRead | Access::StorageRead | Access::StorageWrite => {
                Some(BufferUsages::STORAGE)
            }
            Access::CopySrc => Some(BufferUsages::COPY_SRC),
            Access::CopyDst => Some(BufferUsages::COPY_DST),
        }
    }
}

#[derive(Debug)]
enum Kind {
    Texture(TextureDesc),
    Buffer(BufferDesc),
}

#[derive(Debug)]
struct Resource {
    name: &'static str,
    kind: Kind,
    imported: bool,
}

#[derive(Debug, Default)]
struct Pass {
    name: &'static str,
    /// Resource index and access, in declaration order.
    reads: Vec<(u32, Access)>,
    writes: Vec<(u32, Access)>,
    /// Resources this pass reads and writes on purpose (see [`PassBuilder::read_write`]).
    read_writes: Vec<u32>,
    side_effects: bool,
}

/// Declares what one pass reads and writes.
pub struct PassBuilder<'a> {
    pass: &'a mut Pass,
}

impl PassBuilder<'_> {
    /// The pass samples or copies from `tex` ([`Access::ShaderRead`]).
    pub fn read(&mut self, tex: TexHandle) -> &mut Self {
        self.read_as(tex, Access::ShaderRead)
    }

    /// The pass renders or copies into `tex` ([`Access::ColorTarget`]).
    pub fn write(&mut self, tex: TexHandle) -> &mut Self {
        self.write_as(tex, Access::ColorTarget)
    }

    /// The pass reads `tex` with the given access.
    pub fn read_as(&mut self, tex: TexHandle, access: Access) -> &mut Self {
        self.pass.reads.push((tex.0, access));
        self
    }

    /// The pass writes `tex` with the given access.
    pub fn write_as(&mut self, tex: TexHandle, access: Access) -> &mut Self {
        self.pass.writes.push((tex.0, access));
        self
    }

    /// The pass reads and writes `tex` on purpose (a history texture updated in place).
    /// Without this marker, one pass using a texture as both input and output is an error.
    pub fn read_write(&mut self, tex: TexHandle) -> &mut Self {
        self.pass.read_writes.push(tex.0);
        self.read(tex).write(tex)
    }

    /// The pass reads `buf` ([`Access::ShaderRead`]).
    pub fn read_buf(&mut self, buf: BufHandle) -> &mut Self {
        self.read_buf_as(buf, Access::ShaderRead)
    }

    /// The pass writes `buf` ([`Access::StorageWrite`]).
    pub fn write_buf(&mut self, buf: BufHandle) -> &mut Self {
        self.write_buf_as(buf, Access::StorageWrite)
    }

    /// The pass reads `buf` with the given access.
    pub fn read_buf_as(&mut self, buf: BufHandle, access: Access) -> &mut Self {
        self.pass.reads.push((buf.0, access));
        self
    }

    /// The pass writes `buf` with the given access.
    pub fn write_buf_as(&mut self, buf: BufHandle, access: Access) -> &mut Self {
        self.pass.writes.push((buf.0, access));
        self
    }

    /// The pass must run even if no other pass consumes its output (readback, debug draw).
    pub fn side_effects(&mut self) -> &mut Self {
        self.pass.side_effects = true;
        self
    }
}

/// Problems found while compiling a graph.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// A pass reads a transient resource that no earlier pass wrote.
    #[error("pass `{pass}` reads `{resource}` before any pass writes it")]
    ReadBeforeWrite {
        /// Name of the reading pass.
        pass: &'static str,
        /// Name of the resource.
        resource: &'static str,
    },
    /// A handle does not belong to this graph.
    #[error("pass `{pass}` uses a resource handle from another graph")]
    ForeignHandle {
        /// Name of the offending pass.
        pass: &'static str,
    },
    /// A pass reads and writes the same resource without [`PassBuilder::read_write`].
    #[error("pass `{pass}` reads and writes `{resource}` without declaring read_write")]
    SelfDependency {
        /// Name of the offending pass.
        pass: &'static str,
        /// Name of the resource.
        resource: &'static str,
    },
    /// A buffer is accessed in a way buffers do not support (attachment access).
    #[error("pass `{pass}` uses buffer `{resource}` as {access:?}")]
    InvalidAccess {
        /// Name of the offending pass.
        pass: &'static str,
        /// Name of the buffer.
        resource: &'static str,
        /// The access that buffers do not support.
        access: Access,
    },
    /// A texture's declared usage does not cover what the passes need.
    #[error("texture `{resource}` needs {needed:?} but is declared with {declared:?}")]
    MissingUsage {
        /// Name of the texture.
        resource: &'static str,
        /// Usages required by the passes.
        needed: TextureUsages,
        /// Usages the texture was declared with.
        declared: TextureUsages,
    },
    /// A buffer's declared usage does not cover what the passes need.
    #[error("buffer `{resource}` needs {needed:?} but is declared with {declared:?}")]
    MissingBufferUsage {
        /// Name of the buffer.
        resource: &'static str,
        /// Usages required by the passes.
        needed: BufferUsages,
        /// Usages the buffer was declared with.
        declared: BufferUsages,
    },
}

/// A change of access to one resource between two live passes, as wgpu will see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    /// Name of the resource.
    pub resource: &'static str,
    /// Access left by the previous user.
    pub from: Access,
    /// Access the next pass needs.
    pub to: Access,
    /// The pass that needs the new access.
    pub before_pass: PassId,
}

/// Record of the resources a pass actually touched while it ran. Filled by the renderer (or a
/// test) and checked against the declaration with [`CompiledGraph::verify_usage`].
#[derive(Clone, Debug, Default)]
pub struct UsageLog {
    touched: Vec<(PassId, u32)>,
}

impl UsageLog {
    /// Creates an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `pass` touched texture `tex`.
    pub fn record_texture(&mut self, pass: PassId, tex: TexHandle) {
        self.touched.push((pass, tex.0));
    }

    /// Records that `pass` touched buffer `buf`.
    pub fn record_buffer(&mut self, pass: PassId, buf: BufHandle) {
        self.touched.push((pass, buf.0));
    }
}

/// A pass whose actual resource use differs from its declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessMismatch {
    /// The pass touched a resource it did not declare.
    Undeclared {
        /// Name of the pass.
        pass: &'static str,
        /// Name of the resource.
        resource: &'static str,
    },
    /// The pass declared a resource it never touched.
    Unused {
        /// Name of the pass.
        pass: &'static str,
        /// Name of the resource.
        resource: &'static str,
    },
}

/// A set of passes and resources to compile.
#[derive(Debug, Default)]
pub struct RenderGraph {
    resources: Vec<Resource>,
    passes: Vec<Pass>,
}

impl RenderGraph {
    /// Creates an empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a texture owned by the graph (eligible for memory aliasing).
    pub fn create_texture(&mut self, name: &'static str, desc: TextureDesc) -> TexHandle {
        TexHandle(self.add_resource(name, Kind::Texture(desc), false))
    }

    /// Declares a texture that lives outside the graph (swapchain, history buffer, VSM pool).
    /// Writing it keeps a pass alive; it is never aliased.
    pub fn import_texture(&mut self, name: &'static str, desc: TextureDesc) -> TexHandle {
        TexHandle(self.add_resource(name, Kind::Texture(desc), true))
    }

    /// Declares a buffer owned by the graph (cluster lists, light grids).
    pub fn create_buffer(&mut self, name: &'static str, desc: BufferDesc) -> BufHandle {
        BufHandle(self.add_resource(name, Kind::Buffer(desc), false))
    }

    /// Declares a buffer that lives outside the graph.
    pub fn import_buffer(&mut self, name: &'static str, desc: BufferDesc) -> BufHandle {
        BufHandle(self.add_resource(name, Kind::Buffer(desc), true))
    }

    fn add_resource(&mut self, name: &'static str, kind: Kind, imported: bool) -> u32 {
        self.resources.push(Resource {
            name,
            kind,
            imported,
        });
        self.resources.len() as u32 - 1
    }

    /// Adds a pass; `declare` states its reads and writes.
    pub fn add_pass(
        &mut self,
        name: &'static str,
        declare: impl FnOnce(&mut PassBuilder),
    ) -> PassId {
        let mut pass = Pass {
            name,
            ..Pass::default()
        };
        declare(&mut PassBuilder { pass: &mut pass });
        self.passes.push(pass);
        PassId(self.passes.len() - 1)
    }

    /// Number of declared passes.
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    /// Orders the passes, culls unused ones, checks usage flags and plans texture memory.
    pub fn compile(&self) -> Result<CompiledGraph, GraphError> {
        let (required_usage, required_buffer_usage) = self.validate()?;
        let deps = self.dependencies();
        let live = self.live_passes(&deps);
        let order = self.topological_order(&deps, &live);
        let lifetimes = self.lifetimes(&order);
        let (physical, slot_count) = self.assign_slots(&lifetimes);
        let barrier_plan = self.barrier_plan(&order);
        let critical_path_len = critical_path_len(&order, &deps, &live);
        let culled = (0..self.passes.len())
            .filter(|&i| !live[i])
            .map(PassId)
            .collect();
        let pass_uses = self
            .passes
            .iter()
            .map(|p| p.reads.iter().chain(&p.writes).copied().collect())
            .collect();
        let edges = deps
            .into_iter()
            .enumerate()
            .flat_map(|(to, from)| from.into_iter().map(move |f| (f, to)))
            .collect();
        Ok(CompiledGraph {
            order,
            culled,
            lifetimes,
            physical,
            slot_count,
            required_usage,
            required_buffer_usage,
            barrier_plan,
            critical_path_len,
            pass_uses,
            pass_names: self.passes.iter().map(|p| p.name).collect(),
            resource_names: self.resources.iter().map(|r| r.name).collect(),
            edges,
        })
    }

    fn resource(&self, pass: &'static str, index: u32) -> Result<&Resource, GraphError> {
        self.resources
            .get(index as usize)
            .ok_or(GraphError::ForeignHandle { pass })
    }

    /// Checks handles, access kinds, reads-before-writes, self-dependencies and usage flags.
    /// Returns the usage every texture and buffer needs (indexed by resource).
    fn validate(&self) -> Result<(Vec<TextureUsages>, Vec<BufferUsages>), GraphError> {
        let res_count = self.resources.len();
        let mut written = vec![false; res_count];
        for pass in &self.passes {
            for &(r, access) in pass.reads.iter().chain(&pass.writes) {
                let res = self.resource(pass.name, r)?;
                if matches!(res.kind, Kind::Buffer(_)) && access.buffer_usage().is_none() {
                    return Err(GraphError::InvalidAccess {
                        pass: pass.name,
                        resource: res.name,
                        access,
                    });
                }
            }
            for &(r, _) in &pass.reads {
                let res = &self.resources[r as usize];
                if !res.imported && !written[r as usize] {
                    return Err(GraphError::ReadBeforeWrite {
                        pass: pass.name,
                        resource: res.name,
                    });
                }
            }
            for &(r, _) in &pass.writes {
                let is_read = pass.reads.iter().any(|&(x, _)| x == r);
                if is_read && !pass.read_writes.contains(&r) {
                    return Err(GraphError::SelfDependency {
                        pass: pass.name,
                        resource: self.resources[r as usize].name,
                    });
                }
            }
            for &(r, _) in &pass.writes {
                written[r as usize] = true;
            }
        }

        let mut required_tex = vec![TextureUsages::empty(); res_count];
        let mut required_buf = vec![BufferUsages::empty(); res_count];
        for pass in &self.passes {
            for &(r, access) in pass.reads.iter().chain(&pass.writes) {
                let i = r as usize;
                match self.resources[i].kind {
                    Kind::Texture(_) => required_tex[i] |= access.texture_usage(),
                    Kind::Buffer(_) => {
                        if let Some(usage) = access.buffer_usage() {
                            required_buf[i] |= usage;
                        }
                    }
                }
            }
        }
        for (i, res) in self.resources.iter().enumerate() {
            match &res.kind {
                Kind::Texture(desc) if !desc.usage.contains(required_tex[i]) => {
                    return Err(GraphError::MissingUsage {
                        resource: res.name,
                        needed: required_tex[i],
                        declared: desc.usage,
                    });
                }
                Kind::Buffer(desc) if !desc.usage.contains(required_buf[i]) => {
                    return Err(GraphError::MissingBufferUsage {
                        resource: res.name,
                        needed: required_buf[i],
                        declared: desc.usage,
                    });
                }
                _ => {}
            }
        }
        Ok((required_tex, required_buf))
    }

    /// Dependency edges (earlier -> later), derived from declaration order.
    fn dependencies(&self) -> Vec<BTreeSet<usize>> {
        let n = self.passes.len();
        let res_count = self.resources.len();
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        let mut last_writer: Vec<Option<usize>> = vec![None; res_count];
        let mut readers_since_write: Vec<Vec<usize>> = vec![Vec::new(); res_count];
        for (i, pass) in self.passes.iter().enumerate() {
            for &(r, _) in &pass.reads {
                let r = r as usize;
                if let Some(w) = last_writer[r] {
                    deps[i].insert(w);
                }
                readers_since_write[r].push(i);
            }
            for &(r, _) in &pass.writes {
                let r = r as usize;
                if let Some(w) = last_writer[r] {
                    deps[i].insert(w); // write-after-write
                }
                for &reader in &readers_since_write[r] {
                    if reader != i {
                        deps[i].insert(reader); // write-after-read
                    }
                }
                readers_since_write[r].clear();
                last_writer[r] = Some(i);
            }
            deps[i].remove(&i);
        }
        deps
    }

    /// Passes that must run, plus everything they transitively depend on.
    fn live_passes(&self, deps: &[BTreeSet<usize>]) -> Vec<bool> {
        let n = self.passes.len();
        let mut live = vec![false; n];
        let mut stack: Vec<usize> = (0..n)
            .filter(|&i| {
                let p = &self.passes[i];
                p.side_effects
                    || p.writes
                        .iter()
                        .any(|&(r, _)| self.resources[r as usize].imported)
            })
            .collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut live[i], true) {
                continue;
            }
            stack.extend(deps[i].iter().copied().filter(|&d| !live[d]));
        }
        live
    }

    /// Stable topological order of the live passes (lowest declaration index first).
    fn topological_order(&self, deps: &[BTreeSet<usize>], live: &[bool]) -> Vec<PassId> {
        let n = self.passes.len();
        let mut indegree = vec![0usize; n];
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
        for i in (0..n).filter(|&i| live[i]) {
            for &d in deps[i].iter().filter(|&&d| live[d]) {
                indegree[i] += 1;
                dependents[d].push(i);
            }
        }
        let mut ready: BTreeSet<usize> = (0..n).filter(|&i| live[i] && indegree[i] == 0).collect();
        let mut order = Vec::new();
        while let Some(i) = ready.pop_first() {
            order.push(PassId(i));
            for &next in &dependents[i] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.insert(next);
                }
            }
        }
        order
    }

    /// First and last position in `order` that touches each resource.
    fn lifetimes(&self, order: &[PassId]) -> Vec<Option<(usize, usize)>> {
        let mut lifetimes: Vec<Option<(usize, usize)>> = vec![None; self.resources.len()];
        for (pos, PassId(i)) in order.iter().enumerate() {
            let pass = &self.passes[*i];
            for &(r, _) in pass.reads.iter().chain(&pass.writes) {
                let slot = &mut lifetimes[r as usize];
                *slot = Some(match *slot {
                    None => (pos, pos),
                    Some((first, last)) => (first.min(pos), last.max(pos)),
                });
            }
        }
        lifetimes
    }

    /// Greedy slot assignment: reuse a slot whose previous tenant is already dead and
    /// whose descriptor matches. Only owned textures are assigned.
    fn assign_slots(&self, lifetimes: &[Option<(usize, usize)>]) -> (Vec<Option<usize>>, usize) {
        let res_count = self.resources.len();
        let mut by_first: Vec<usize> = (0..res_count)
            .filter(|&r| !self.resources[r].imported && lifetimes[r].is_some())
            .filter(|&r| matches!(self.resources[r].kind, Kind::Texture(_)))
            .collect();
        by_first.sort_by_key(|&r| (lifetimes[r].map(|l| l.0), r));
        let mut slots: Vec<(TextureDesc, usize)> = Vec::new(); // (desc, last use)
        let mut physical: Vec<Option<usize>> = vec![None; res_count];
        for r in by_first {
            let (first, last) = lifetimes[r].unwrap_or((0, 0));
            let Kind::Texture(desc) = self.resources[r].kind else {
                continue;
            };
            let reuse = slots
                .iter()
                .position(|(d, free_after)| *d == desc && *free_after < first);
            let slot = match reuse {
                Some(s) => {
                    slots[s].1 = last;
                    s
                }
                None => {
                    slots.push((desc, last));
                    slots.len() - 1
                }
            };
            physical[r] = Some(slot);
        }
        (physical, slots.len())
    }

    /// Access changes between live passes, in execution order. A pass is entered with the
    /// access of its first use of a resource (reads before writes) and leaves it with its last.
    fn barrier_plan(&self, order: &[PassId]) -> Vec<Transition> {
        let mut state: Vec<Option<Access>> = vec![None; self.resources.len()];
        let mut plan = Vec::new();
        for &PassId(i) in order {
            let pass = &self.passes[i];
            let mut entered: Vec<(u32, Access)> = Vec::new();
            for &(r, access) in pass.reads.iter().chain(&pass.writes) {
                if !entered.iter().any(|&(e, _)| e == r) {
                    entered.push((r, access));
                }
            }
            for (r, access) in entered {
                let previous = state[r as usize];
                if let Some(from) = previous.filter(|&from| from != access) {
                    plan.push(Transition {
                        resource: self.resources[r as usize].name,
                        from,
                        to: access,
                        before_pass: PassId(i),
                    });
                }
            }
            for &(r, access) in pass.reads.iter().chain(&pass.writes) {
                state[r as usize] = Some(access);
            }
        }
        plan
    }
}

/// Length of the longest dependency chain among the live passes, counted in passes.
fn critical_path_len(order: &[PassId], deps: &[BTreeSet<usize>], live: &[bool]) -> usize {
    let mut depth = vec![0usize; live.len()];
    let mut best = 0;
    for &PassId(i) in order {
        let before = deps[i]
            .iter()
            .filter(|&&d| live[d])
            .map(|&d| depth[d])
            .max()
            .unwrap_or(0);
        depth[i] = before + 1;
        best = best.max(depth[i]);
    }
    best
}

/// Result of [`RenderGraph::compile`].
#[derive(Clone, Debug)]
pub struct CompiledGraph {
    /// Live passes in execution order.
    pub order: Vec<PassId>,
    /// Passes that were dropped because nothing needs their output.
    pub culled: Vec<PassId>,
    /// For each resource (by handle index): first and last position in `order` that touches
    /// it, or `None` when no live pass does.
    pub lifetimes: Vec<Option<(usize, usize)>>,
    /// For each transient texture: the physical slot it lives in (`None` for imported or
    /// unused textures and for buffers).
    pub physical: Vec<Option<usize>>,
    /// Number of physical textures that must be allocated for the transient ones.
    pub slot_count: usize,
    /// Usage flags each texture needs, by resource index (empty for buffers).
    required_usage: Vec<TextureUsages>,
    /// Usage flags each buffer needs, by resource index (empty for textures).
    required_buffer_usage: Vec<BufferUsages>,
    /// Access changes between live passes, for reports and tests.
    pub barrier_plan: Vec<Transition>,
    /// Number of passes on the longest dependency chain of the live passes.
    pub critical_path_len: usize,
    /// Declared accesses of every pass (all passes, culled ones included), by declaration index.
    pass_uses: Vec<Vec<(u32, Access)>>,
    pass_names: Vec<&'static str>,
    resource_names: Vec<&'static str>,
    edges: Vec<(usize, usize)>,
}

impl CompiledGraph {
    /// Physical slot of a transient texture.
    pub fn slot_of(&self, tex: TexHandle) -> Option<usize> {
        self.physical.get(tex.0 as usize).copied().flatten()
    }

    /// Usage flags the passes need for a texture (union over all declared passes).
    pub fn required_usage(&self, tex: TexHandle) -> TextureUsages {
        self.required_usage
            .get(tex.0 as usize)
            .copied()
            .unwrap_or(wgpu::TextureUsages::empty())
    }

    /// Usage flags the passes need for a buffer (union over all declared passes).
    pub fn required_buffer_usage(&self, buf: BufHandle) -> BufferUsages {
        self.required_buffer_usage
            .get(buf.0 as usize)
            .copied()
            .unwrap_or(wgpu::BufferUsages::empty())
    }

    /// Name of a pass.
    pub fn pass_name(&self, pass: PassId) -> &'static str {
        self.pass_names[pass.0]
    }

    /// Name of a texture.
    pub fn texture_name(&self, tex: TexHandle) -> &'static str {
        self.resource_names[tex.0 as usize]
    }

    /// Name of a buffer.
    pub fn buffer_name(&self, buf: BufHandle) -> &'static str {
        self.resource_names[buf.0 as usize]
    }

    /// Checks what the passes actually touched against their declarations. Culled passes are
    /// not expected to run, so they are not checked.
    pub fn verify_usage(&self, log: &UsageLog) -> Vec<AccessMismatch> {
        let mut out = Vec::new();
        let name_of = |r: u32| {
            self.resource_names
                .get(r as usize)
                .copied()
                .unwrap_or("<unknown>")
        };
        for &PassId(i) in &self.order {
            let pass = self.pass_names[i];
            let declared: BTreeSet<u32> = self.pass_uses[i].iter().map(|&(r, _)| r).collect();
            let touched: BTreeSet<u32> = log
                .touched
                .iter()
                .filter(|(p, _)| p.0 == i)
                .map(|&(_, r)| r)
                .collect();
            for &r in touched.difference(&declared) {
                out.push(AccessMismatch::Undeclared {
                    pass,
                    resource: name_of(r),
                });
            }
            for &r in declared.difference(&touched) {
                out.push(AccessMismatch::Unused {
                    pass,
                    resource: name_of(r),
                });
            }
        }
        out
    }

    /// Human-readable description of the compiled frame: passes in order with their resources,
    /// slots, culled passes and the access changes wgpu will perform.
    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} live passes, {} culled, {} transient slots, critical path {}",
            self.order.len(),
            self.culled.len(),
            self.slot_count,
            self.critical_path_len
        );
        for (pos, &PassId(i)) in self.order.iter().enumerate() {
            let _ = writeln!(out, "{}. {}", pos, self.pass_names[i]);
            for &(r, access) in &self.pass_uses[i] {
                let _ = writeln!(
                    out,
                    "    {:?} {} ({})",
                    access,
                    self.resource_names[r as usize],
                    self.resource_note(r as usize)
                );
            }
        }
        if !self.culled.is_empty() {
            let names: Vec<_> = self.culled.iter().map(|p| self.pass_names[p.0]).collect();
            let _ = writeln!(out, "culled: {}", names.join(", "));
        }
        for t in &self.barrier_plan {
            let _ = writeln!(
                out,
                "barrier {}: {:?} -> {:?} before {}",
                t.resource, t.from, t.to, self.pass_names[t.before_pass.0]
            );
        }
        out
    }

    fn resource_note(&self, r: usize) -> String {
        match (self.physical[r], self.lifetimes[r]) {
            (Some(slot), Some((first, last))) => format!("slot {slot}, live {first}..={last}"),
            (None, Some((first, last))) => format!("imported or buffer, live {first}..={last}"),
            _ => "unused".to_string(),
        }
    }

    /// Graphviz `dot` description of the live passes and their dependencies, for debugging.
    pub fn to_dot(&self) -> String {
        let mut out = String::from("digraph frame {\n  rankdir=LR;\n");
        for (pos, id) in self.order.iter().enumerate() {
            let _ = writeln!(
                out,
                "  p{} [label=\"{}. {}\"];",
                id.0, pos, self.pass_names[id.0]
            );
        }
        for id in &self.culled {
            let _ = writeln!(
                out,
                "  p{} [label=\"{} (culled)\", style=dashed];",
                id.0, self.pass_names[id.0]
            );
        }
        for (from, to) in &self.edges {
            let _ = writeln!(out, "  p{from} -> p{to};");
        }
        out.push_str("}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(w: u32) -> TextureDesc {
        TextureDesc {
            width: w,
            height: w,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        }
    }

    fn buf(size: u64) -> BufferDesc {
        BufferDesc {
            size,
            usage: BufferUsages::STORAGE,
        }
    }

    fn names(g: &CompiledGraph) -> Vec<&'static str> {
        g.order.iter().map(|&p| g.pass_name(p)).collect()
    }

    #[test]
    fn linear_chain_runs_in_order() {
        let mut g = RenderGraph::new();
        let hdr = g.create_texture("hdr", desc(64));
        let ldr = g.create_texture("ldr", desc(64));
        let screen = g.import_texture("screen", desc(64));
        g.add_pass("forward", |p| {
            p.write(hdr);
        });
        g.add_pass("bloom", |p| {
            p.read(hdr).write(ldr);
        });
        g.add_pass("tonemap", |p| {
            p.read(ldr).write(screen);
        });
        let c = g.compile().unwrap();
        assert_eq!(names(&c), ["forward", "bloom", "tonemap"]);
        assert!(c.culled.is_empty());
    }

    #[test]
    fn unused_passes_are_culled_transitively() {
        let mut g = RenderGraph::new();
        let a = g.create_texture("a", desc(8));
        let b = g.create_texture("b", desc(8));
        let used = g.create_texture("used", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("makes_a", |p| {
            p.write(a);
        });
        g.add_pass("makes_b_from_a", |p| {
            p.read(a).write(b);
        });
        g.add_pass("makes_used", |p| {
            p.write(used);
        });
        g.add_pass("present", |p| {
            p.read(used).write(screen);
        });
        let c = g.compile().unwrap();
        assert_eq!(names(&c), ["makes_used", "present"]);
        assert_eq!(c.culled, [PassId(0), PassId(1)]);
        assert_eq!(c.lifetimes[a.0 as usize], None, "no live pass touches `a`");
        assert_eq!(c.slot_of(a), None);
    }

    #[test]
    fn side_effect_passes_are_kept() {
        let mut g = RenderGraph::new();
        let t = g.create_texture("t", desc(8));
        g.add_pass("readback", |p| {
            p.write(t).side_effects();
        });
        assert_eq!(names(&g.compile().unwrap()), ["readback"]);
    }

    #[test]
    fn reading_an_unwritten_transient_texture_is_an_error() {
        let mut g = RenderGraph::new();
        let t = g.create_texture("t", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("bad", |p| {
            p.read(t).write(screen);
        });
        assert_eq!(
            g.compile().unwrap_err(),
            GraphError::ReadBeforeWrite {
                pass: "bad",
                resource: "t"
            }
        );
    }

    #[test]
    fn imported_textures_may_be_read_without_a_writer() {
        let mut g = RenderGraph::new();
        let history = g.import_texture("taa_history", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("taa", |p| {
            p.read(history).write(screen);
        });
        assert!(g.compile().is_ok());
    }

    #[test]
    fn write_after_read_keeps_declaration_order() {
        let mut g = RenderGraph::new();
        let shared = g.import_texture("shared", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("consume_old", |p| {
            p.read(shared).write(screen);
        });
        g.add_pass("overwrite", |p| {
            p.write(shared).side_effects();
        });
        assert_eq!(names(&g.compile().unwrap()), ["consume_old", "overwrite"]);
    }

    #[test]
    fn disjoint_lifetimes_share_a_slot() {
        let mut g = RenderGraph::new();
        let a = g.create_texture("a", desc(32));
        let b = g.create_texture("b", desc(32));
        let c = g.create_texture("c", desc(32));
        let screen = g.import_texture("screen", desc(32));
        g.add_pass("p1", |p| {
            p.write(a);
        });
        g.add_pass("p2", |p| {
            p.read(a).write(b);
        });
        g.add_pass("p3", |p| {
            p.read(b).write(c);
        });
        g.add_pass("p4", |p| {
            p.read(c).write(screen);
        });
        let comp = g.compile().unwrap();
        // a: 0..1, b: 1..2, c: 2..3 -> a and c never overlap and have the same descriptor.
        assert_eq!(comp.slot_of(a), comp.slot_of(c));
        assert_ne!(comp.slot_of(a), comp.slot_of(b));
        assert_eq!(comp.slot_count, 2);
    }

    #[test]
    fn different_descriptors_never_alias() {
        let mut g = RenderGraph::new();
        let a = g.create_texture("a", desc(32));
        let b = g.create_texture("b", desc(16));
        let screen = g.import_texture("screen", desc(32));
        g.add_pass("p1", |p| {
            p.write(a);
        });
        g.add_pass("p2", |p| {
            p.read(a).write(screen);
        });
        g.add_pass("p3", |p| {
            p.write(b);
        });
        g.add_pass("p4", |p| {
            p.read(b).write(screen);
        });
        let comp = g.compile().unwrap();
        assert_ne!(comp.slot_of(a), comp.slot_of(b));
        assert_eq!(comp.slot_count, 2);
    }

    #[test]
    fn dot_output_lists_passes_and_edges() {
        let mut g = RenderGraph::new();
        let a = g.create_texture("a", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("first", |p| {
            p.write(a);
        });
        g.add_pass("second", |p| {
            p.read(a).write(screen);
        });
        g.add_pass("dead", |p| {
            p.write(a);
        });
        let dot = g.compile().unwrap().to_dot();
        assert!(dot.contains("first") && dot.contains("second"));
        assert!(dot.contains("p0 -> p1"));
        assert!(dot.contains("(culled)"));
    }

    #[test]
    fn the_usage_flags_are_checked_against_the_declared_access() {
        let mut g = RenderGraph::new();
        let t = g.create_texture("t", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("writer", |p| {
            p.write_as(t, Access::StorageWrite);
        });
        g.add_pass("present", |p| {
            p.read(t).write(screen);
        });
        assert_eq!(
            g.compile().unwrap_err(),
            GraphError::MissingUsage {
                resource: "t",
                needed: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                declared: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            }
        );
    }

    #[test]
    fn a_buffer_dependency_orders_the_passes() {
        let mut g = RenderGraph::new();
        let clusters = g.create_buffer("clusters", buf(256));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("cull", |p| {
            p.write_buf(clusters);
        });
        g.add_pass("shade", |p| {
            p.read_buf(clusters).write(screen);
        });
        let c = g.compile().unwrap();
        assert_eq!(names(&c), ["cull", "shade"]);
        // The first use of the buffer is not a transition; the write-to-read change is.
        let on_buffer: Vec<_> = c
            .barrier_plan
            .iter()
            .filter(|t| t.resource == "clusters")
            .collect();
        assert_eq!(on_buffer.len(), 1);
        assert_eq!(on_buffer[0].from, Access::StorageWrite);
        assert_eq!(on_buffer[0].to, Access::ShaderRead);
        assert_eq!(c.lifetimes[clusters.0 as usize], Some((0, 1)));
        assert_eq!(c.required_buffer_usage(clusters), BufferUsages::STORAGE);
    }

    #[test]
    fn a_buffer_cannot_be_a_colour_target() {
        let mut g = RenderGraph::new();
        let b = g.create_buffer("b", buf(16));
        g.add_pass("bad", |p| {
            p.write_buf_as(b, Access::ColorTarget);
        });
        assert_eq!(
            g.compile().unwrap_err(),
            GraphError::InvalidAccess {
                pass: "bad",
                resource: "b",
                access: Access::ColorTarget,
            }
        );
    }

    #[test]
    fn a_pass_that_reads_and_writes_the_same_texture_is_reported() {
        let mut g = RenderGraph::new();
        let history = g.import_texture("history", desc(8));
        g.add_pass("in_place", |p| {
            p.read(history).write(history).side_effects();
        });
        assert_eq!(
            g.compile().unwrap_err(),
            GraphError::SelfDependency {
                pass: "in_place",
                resource: "history"
            }
        );

        let mut ok = RenderGraph::new();
        let history = ok.import_texture("history", desc(8));
        ok.add_pass("in_place", |p| {
            p.read_write(history).side_effects();
        });
        assert!(ok.compile().is_ok());
    }

    #[test]
    fn the_barrier_plan_lists_every_change_of_access() {
        let mut g = RenderGraph::new();
        let a = g.create_texture("a", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("draw", |p| {
            p.write(a);
        });
        g.add_pass("present", |p| {
            p.read(a).write(screen);
        });
        let c = g.compile().unwrap();
        assert_eq!(
            c.barrier_plan,
            [Transition {
                resource: "a",
                from: Access::ColorTarget,
                to: Access::ShaderRead,
                before_pass: PassId(1),
            }]
        );
    }

    #[test]
    fn an_unused_buffer_is_culled_with_its_producer() {
        let mut g = RenderGraph::new();
        let grid = g.create_buffer("light_grid", buf(64));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("build_grid", |p| {
            p.write_buf(grid);
        });
        g.add_pass("present", |p| {
            p.write(screen);
        });
        let c = g.compile().unwrap();
        assert_eq!(c.culled, [PassId(0)]);
        assert_eq!(names(&c), ["present"]);
    }

    #[test]
    fn the_critical_path_of_a_chain_equals_its_length() {
        let mut g = RenderGraph::new();
        let a = g.create_texture("a", desc(8));
        let b = g.create_texture("b", desc(8));
        let c = g.create_texture("c", desc(8));
        let screen = g.import_texture("screen", desc(8));
        g.add_pass("p1", |p| {
            p.write(a);
        });
        g.add_pass("p2", |p| {
            p.read(a).write(b);
        });
        g.add_pass("p3", |p| {
            p.read(b).write(c);
        });
        g.add_pass("p4", |p| {
            p.read(c).write(screen);
        });
        assert_eq!(g.compile().unwrap().critical_path_len, 4);
    }

    #[test]
    fn declared_and_touched_resources_must_match() {
        let mut g = RenderGraph::new();
        let a = g.create_texture("a", desc(8));
        let b = g.create_texture("b", desc(8));
        let screen = g.import_texture("screen", desc(8));
        let make = g.add_pass("make", |p| {
            p.write(a).write(b);
        });
        let present = g.add_pass("present", |p| {
            p.read(a).read(b).write(screen);
        });
        let c = g.compile().unwrap();
        let mut log = UsageLog::new();
        // `make` touched `screen` by mistake and never touched `b`; `present` is exact.
        log.record_texture(make, a);
        log.record_texture(make, screen);
        for tex in [a, b, screen] {
            log.record_texture(present, tex);
        }
        assert_eq!(
            c.verify_usage(&log),
            [
                AccessMismatch::Undeclared {
                    pass: "make",
                    resource: "screen"
                },
                AccessMismatch::Unused {
                    pass: "make",
                    resource: "b"
                },
            ]
        );
    }
}
