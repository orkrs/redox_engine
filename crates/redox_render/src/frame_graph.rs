//! The frame as a [`RenderGraph`]: which pass reads and writes which texture.
//!
//! This declaration drives the renderer: [`frame_pass_order`] is the compiled order, and
//! `renderer/frame.rs` runs one method per pass in exactly that order (a test checks that every
//! declared pass has an implementation). Changing the order or adding a pass starts here.
//! Use [`frame_graph`] + [`RenderGraph::compile`] + `to_dot` to print the frame.
//!
//! Dependencies are declared by hand at texture granularity; buffers (cluster lists, light
//! grids) are folded into the textures of the passes that consume them.

use wgpu::{TextureFormat as F, TextureUsages as U};

use crate::render_graph::{RenderGraph, TextureDesc};

fn target(width: u32, height: u32, format: F) -> TextureDesc {
    TextureDesc {
        width,
        height,
        format,
        usage: U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
    }
}

/// Declares the frame for a render target of `width` x `height`.
pub fn frame_graph(width: u32, height: u32) -> RenderGraph {
    let (w, h) = (width.max(1), height.max(1));
    let mut g = RenderGraph::new();

    // Persistent resources that live outside the frame.
    let shadow_maps = g.import_texture("vsm_pool", target(8192, 8192, F::Depth32Float));
    let ddgi = g.import_texture("ddgi_irradiance", target(1024, 1024, F::Rgba16Float));
    let taa_history = g.import_texture("taa_history", target(w, h, F::Rgba16Float));
    let backbuffer = g.import_texture("backbuffer", target(w, h, F::Bgra8UnormSrgb));

    // Per-frame transients.
    let clusters = g.create_texture("vg_clusters", target(w, h, F::R32Uint));
    let depth = g.create_texture("depth", target(w, h, F::Depth32Float));
    let normals = g.create_texture("normals", target(w, h, F::Rgba8Unorm));
    let hiz = g.create_texture("hiz", target(w, h, F::R32Float));
    let ao = g.create_texture("ao", target(w, h, F::R8Unorm));
    let shadow_mask = g.create_texture("shadow_mask", target(w, h, F::R8Unorm));
    let hdr = g.create_texture("hdr_color", target(w, h, F::Rgba16Float));
    let hdr_resolved = g.create_texture("hdr_resolved", target(w, h, F::Rgba16Float));

    g.add_pass("shadow_passes", |p| {
        p.write(shadow_maps);
    });
    g.add_pass("ddgi_fill", |p| {
        p.read(shadow_maps).write(ddgi);
    });
    g.add_pass("cluster_vg_cull", |p| {
        p.write(clusters);
    });
    g.add_pass("normal_prepass_hiz", |p| {
        p.read(clusters).write(depth).write(normals).write(hiz);
    });
    g.add_pass("gtao", |p| {
        p.read(depth).read(normals).read(hiz).write(ao);
    });
    g.add_pass("vsm_screen_shadow", |p| {
        p.read(depth).read(shadow_maps).write(shadow_mask);
    });
    g.add_pass("forward", |p| {
        p.read(ao)
            .read(shadow_mask)
            .read(ddgi)
            .read(depth)
            .write(hdr);
    });
    g.add_pass("vg_visibility_resolve", |p| {
        p.read(clusters).read(depth).read(shadow_mask).write(hdr);
    });
    g.add_pass("transparent", |p| {
        p.read(depth).read(shadow_mask).write(hdr);
    });
    g.add_pass("msaa_resolve_particles", |p| {
        p.read(hdr).read(depth).write(hdr_resolved);
    });
    g.add_pass("scatter_light", |p| {
        p.read(depth).read(shadow_maps).write(hdr_resolved);
    });
    g.add_pass("taa_tonemap_bloom_smaa", |p| {
        p.read(hdr_resolved)
            .read(normals)
            .read(taa_history)
            .write(taa_history)
            .write(backbuffer);
    });
    g.add_pass("frame_tail", |p| {
        p.read(backbuffer).side_effects();
    });
    g
}

/// Names of the frame's passes in execution order, taken from the compiled [`frame_graph`].
///
/// The renderer runs exactly these passes in exactly this order, so editing the graph changes
/// the frame. The order does not depend on the render size.
pub fn frame_pass_order() -> &'static [&'static str] {
    static ORDER: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    ORDER.get_or_init(|| {
        let compiled = frame_graph(1, 1)
            .compile()
            .expect("the frame graph is valid (checked by tests)");
        compiled
            .order
            .iter()
            .map(|&pass| compiled.pass_name(pass))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_SOURCE: &str = include_str!("renderer/frame.rs");

    #[test]
    fn every_declared_pass_has_an_implementation_in_the_renderer() {
        for name in frame_pass_order() {
            assert!(
                FRAME_SOURCE.contains(&format!("\"{name}\" =>")),
                "pass `{name}` is declared in frame_graph.rs but renderer/frame.rs does not run it"
            );
        }
    }

    #[test]
    fn execution_order_is_the_compiled_graph_order() {
        let compiled = frame_graph(1920, 1080).compile().unwrap();
        let declared: Vec<_> = compiled
            .order
            .iter()
            .map(|&p| compiled.pass_name(p))
            .collect();
        assert_eq!(declared, frame_pass_order());
        assert_eq!(declared.first(), Some(&"shadow_passes"));
        assert_eq!(declared.last(), Some(&"frame_tail"));
    }

    #[test]
    fn nothing_is_culled_and_slot_plan_is_valid() {
        let compiled = frame_graph(1920, 1080).compile().unwrap();
        assert!(
            compiled.culled.is_empty(),
            "culled passes: {:?}",
            compiled.culled
        );
        // The frame's transients overlap in time or differ in format (ao and shadow_mask share
        // R8 but live together; hdr and hdr_resolved overlap in the resolve pass), so today the
        // plan needs one slot each. A future format merge would lower this number.
        let transients = 8;
        assert!(compiled.slot_count <= transients);
        assert_eq!(
            compiled.physical.iter().flatten().count(),
            transients,
            "every transient gets a slot"
        );
    }

    #[test]
    fn dot_export_names_every_pass() {
        let graph = frame_graph(640, 360);
        let dot = graph.compile().unwrap().to_dot();
        for name in frame_pass_order() {
            assert!(dot.contains(name), "{name} missing from the DOT output");
        }
    }
}
