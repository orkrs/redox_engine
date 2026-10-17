//! Render graph compiler: declares passes with the textures they read and write, then
//! orders them, drops the ones nobody needs, computes how long each transient texture lives
//! and assigns physical slots so textures with disjoint lifetimes share memory.
//!
//! This module is pure CPU logic (no GPU calls). The renderer takes the pass *order* from it
//! (see [`crate::frame_graph`]); passes still own their textures, so the aliasing plan is
//! informational. wgpu already inserts resource barriers, so the graph's value is explicit
//! dependencies, culling, a single source of truth for the pass order and uniform profiling
//! labels - not barriers.
//!
//! # Semantics
//! * Passes are declared in a sensible sequential order. A pass that reads a texture depends
//!   on the **last earlier pass that wrote it**; a pass that writes a texture depends on
//!   earlier readers and writers of it (so declaration order resolves write-after-read).
//! * Reading a transient texture nobody wrote earlier is an error
//!   ([`GraphError::ReadBeforeWrite`]); imported (external) textures may be read freely.
//! * A pass is kept if it has side effects, writes an imported texture, or writes something a
//!   kept pass reads. Everything else is culled.

use std::collections::BTreeSet;
use std::fmt::Write as _;

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
    pub usage: wgpu::TextureUsages,
}

/// Handle of a texture declared in a [`RenderGraph`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TexHandle(u32);

/// Index of a pass in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PassId(pub usize);

#[derive(Debug)]
struct Resource {
    name: &'static str,
    desc: TextureDesc,
    imported: bool,
}

#[derive(Debug, Default)]
struct Pass {
    name: &'static str,
    reads: Vec<TexHandle>,
    writes: Vec<TexHandle>,
    side_effects: bool,
}

/// Declares what one pass reads and writes.
pub struct PassBuilder<'a> {
    pass: &'a mut Pass,
}

impl PassBuilder<'_> {
    /// The pass samples or copies from `tex`.
    pub fn read(&mut self, tex: TexHandle) -> &mut Self {
        self.pass.reads.push(tex);
        self
    }

    /// The pass renders or copies into `tex`.
    pub fn write(&mut self, tex: TexHandle) -> &mut Self {
        self.pass.writes.push(tex);
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
    /// A pass reads a transient texture that no earlier pass wrote.
    #[error("pass `{pass}` reads `{texture}` before any pass writes it")]
    ReadBeforeWrite {
        /// Name of the reading pass.
        pass: &'static str,
        /// Name of the texture.
        texture: &'static str,
    },
    /// A handle does not belong to this graph.
    #[error("pass `{pass}` uses a texture handle from another graph")]
    ForeignHandle {
        /// Name of the offending pass.
        pass: &'static str,
    },
}

/// A set of passes and textures to compile.
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
        self.add_resource(name, desc, false)
    }

    /// Declares a texture that lives outside the graph (swapchain, history buffer, VSM pool).
    /// Writing it keeps a pass alive; it is never aliased.
    pub fn import_texture(&mut self, name: &'static str, desc: TextureDesc) -> TexHandle {
        self.add_resource(name, desc, true)
    }

    fn add_resource(&mut self, name: &'static str, desc: TextureDesc, imported: bool) -> TexHandle {
        self.resources.push(Resource {
            name,
            desc,
            imported,
        });
        TexHandle(self.resources.len() as u32 - 1)
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

    /// Orders the passes, culls unused ones and plans texture memory.
    pub fn compile(&self) -> Result<CompiledGraph, GraphError> {
        let n = self.passes.len();
        let res_count = self.resources.len();

        // Validate handles and reads-before-writes.
        let mut written = vec![false; res_count];
        for pass in &self.passes {
            for h in pass.reads.iter().chain(&pass.writes) {
                if h.0 as usize >= res_count {
                    return Err(GraphError::ForeignHandle { pass: pass.name });
                }
            }
            for h in &pass.reads {
                let r = &self.resources[h.0 as usize];
                if !r.imported && !written[h.0 as usize] {
                    return Err(GraphError::ReadBeforeWrite {
                        pass: pass.name,
                        texture: r.name,
                    });
                }
            }
            for h in &pass.writes {
                written[h.0 as usize] = true;
            }
        }

        // Dependency edges (earlier -> later), derived from declaration order.
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        let mut last_writer: Vec<Option<usize>> = vec![None; res_count];
        let mut readers_since_write: Vec<Vec<usize>> = vec![Vec::new(); res_count];
        for (i, pass) in self.passes.iter().enumerate() {
            for h in &pass.reads {
                let r = h.0 as usize;
                if let Some(w) = last_writer[r] {
                    deps[i].insert(w);
                }
                readers_since_write[r].push(i);
            }
            for h in &pass.writes {
                let r = h.0 as usize;
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

        // Culling: walk backwards from passes that must run.
        let mut live = vec![false; n];
        let mut stack: Vec<usize> = (0..n)
            .filter(|&i| {
                let p = &self.passes[i];
                p.side_effects
                    || p.writes
                        .iter()
                        .any(|h| self.resources[h.0 as usize].imported)
            })
            .collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut live[i], true) {
                continue;
            }
            stack.extend(deps[i].iter().copied().filter(|&d| !live[d]));
        }

        // Stable topological order of the live passes (lowest declaration index first).
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

        // Lifetimes of transient textures in execution order.
        let mut lifetimes: Vec<Option<(usize, usize)>> = vec![None; res_count];
        for (pos, PassId(i)) in order.iter().enumerate() {
            let pass = &self.passes[*i];
            for h in pass.reads.iter().chain(&pass.writes) {
                let slot = &mut lifetimes[h.0 as usize];
                *slot = Some(match *slot {
                    None => (pos, pos),
                    Some((first, last)) => (first.min(pos), last.max(pos)),
                });
            }
        }

        // Greedy slot assignment: reuse a slot whose previous tenant is already dead and
        // whose descriptor matches.
        let mut by_first: Vec<usize> = (0..res_count)
            .filter(|&r| !self.resources[r].imported && lifetimes[r].is_some())
            .collect();
        by_first.sort_by_key(|&r| (lifetimes[r].map(|l| l.0), r));
        let mut slots: Vec<(TextureDesc, usize)> = Vec::new(); // (desc, last use)
        let mut physical: Vec<Option<usize>> = vec![None; res_count];
        for r in by_first {
            let (first, last) = lifetimes[r].unwrap_or((0, 0));
            let desc = self.resources[r].desc;
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

        let culled = (0..n).filter(|&i| !live[i]).map(PassId).collect();
        Ok(CompiledGraph {
            order,
            culled,
            lifetimes,
            physical,
            slot_count: slots.len(),
            pass_names: self.passes.iter().map(|p| p.name).collect(),
            resource_names: self.resources.iter().map(|r| r.name).collect(),
            edges: deps
                .into_iter()
                .enumerate()
                .flat_map(|(to, from)| from.into_iter().map(move |f| (f, to)))
                .collect(),
        })
    }
}

/// Result of [`RenderGraph::compile`].
#[derive(Clone, Debug)]
pub struct CompiledGraph {
    /// Live passes in execution order.
    pub order: Vec<PassId>,
    /// Passes that were dropped because nothing needs their output.
    pub culled: Vec<PassId>,
    /// For each texture (by handle index): first and last position in `order` that touches
    /// it, or `None` when no live pass does.
    pub lifetimes: Vec<Option<(usize, usize)>>,
    /// For each transient texture: the physical slot it lives in (`None` for imported or
    /// unused textures).
    pub physical: Vec<Option<usize>>,
    /// Number of physical textures that must be allocated for the transient ones.
    pub slot_count: usize,
    pass_names: Vec<&'static str>,
    resource_names: Vec<&'static str>,
    edges: Vec<(usize, usize)>,
}

impl CompiledGraph {
    /// Physical slot of a transient texture.
    pub fn slot_of(&self, tex: TexHandle) -> Option<usize> {
        self.physical.get(tex.0 as usize).copied().flatten()
    }

    /// Name of a pass.
    pub fn pass_name(&self, pass: PassId) -> &'static str {
        self.pass_names[pass.0]
    }

    /// Name of a texture.
    pub fn texture_name(&self, tex: TexHandle) -> &'static str {
        self.resource_names[tex.0 as usize]
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
                texture: "t"
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
}
