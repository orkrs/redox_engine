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

    // Persistent resources that live outside the frame (owned by the renderer, created at init and
    // resize). Imported so that a read before the first write is not a graph error.
    let shadow_maps = g.import_texture("vsm_pool", target(8192, 8192, F::Depth32Float));
    let gi = g.import_texture("gi_irradiance", target(1024, 1024, F::Rgba16Float));
    let taa_history = g.import_texture("taa_history", target(w, h, F::Rgba16Float));
    let backbuffer = g.import_texture("backbuffer", target(w, h, F::Bgra8UnormSrgb));
    // VSM screen shadow (filtered output of the temporal pass, `vsm_shadow_filtered_view`).
    let vsm_shadow = g.import_texture("vsm_shadow_filtered", target(w, h, F::R8Unorm));
    // Material surface target written by the normal prepass (`material_surface_view`).
    let material_surface = g.import_texture("material_surface", target(w, h, F::Rgba8Unorm));
    // Visibility targets of the virtual geometry (id, normal/depth and uv views, one group here).
    let vg_vis = g.import_texture("vg_vis_targets", target(w, h, F::Rgba16Float));
    // Post-chain intermediates: TAA velocity, LDR after tonemap, SMAA output.
    let taa_velocity = g.import_texture("taa_velocity", target(w, h, F::Rg16Float));
    let ldr = g.import_texture("ldr", target(w, h, F::Rgba8Unorm));
    let smaa_out = g.import_texture("smaa_out", target(w, h, F::Rgba8Unorm));
    // Last frame's Hi-Z pyramid (read by the cull before this frame builds its own) and last
    // frame's depth (TAA disocclusion test). Both persist across frames, hence imports.
    let hiz_prev = g.import_texture("hiz_prev", target(w, h, F::R32Float));
    let prev_depth = g.import_texture("prev_depth", target(w, h, F::Depth32Float));

    // Per-frame transients.
    let clusters = g.create_texture("vg_clusters", target(w, h, F::R32Uint));
    let depth = g.create_texture("depth", target(w, h, F::Depth32Float));
    let normals = g.create_texture("normals", target(w, h, F::Rgba8Unorm));
    let hiz = g.create_texture("hiz", target(w, h, F::R32Float));
    let ao = g.create_texture("ao", target(w, h, F::R8Unorm));
    let hdr = g.create_texture("hdr_color", target(w, h, F::Rgba16Float));

    g.add_pass("shadow_passes", |p| {
        p.write(shadow_maps);
    });
    g.add_pass("gi_update", |p| {
        p.write(gi);
    });
    // The cull reads last frame's Hi-Z (the current one is built later in normal_prepass_hiz).
    g.add_pass("cluster_vg_cull", |p| {
        p.read(hiz_prev).write(clusters);
    });
    g.add_pass("normal_prepass_hiz", |p| {
        p.write(depth)
            .write(normals)
            .write(hiz)
            .write(material_surface);
    });
    // GTAO reads the normals only (raw, blur and upsample share the normal target).
    g.add_pass("gtao", |p| {
        p.read(normals).write(ao);
    });
    g.add_pass("vsm_screen_shadow", |p| {
        p.read(normals).read(shadow_maps).write(vsm_shadow);
    });
    // The lit passes read the scene bind group: clusters, AO, GI, VSM pages and VSM shadow.
    g.add_pass("forward", |p| {
        p.read(clusters)
            .read(ao)
            .read(vsm_shadow)
            .read(gi)
            .read(shadow_maps)
            .read(depth)
            .write(hdr);
    });
    // Writes the visibility targets, then resolves them into HDR in the same pass.
    g.add_pass("vg_visibility_resolve", |p| {
        p.read(clusters)
            .read(depth)
            .read(ao)
            .read(gi)
            .read(shadow_maps)
            .read(vsm_shadow)
            .read_write(vg_vis)
            .write(hdr);
    });
    // Transparent and additive draws use the scene bind group too.
    g.add_pass("transparent", |p| {
        p.read(clusters)
            .read(depth)
            .read(ao)
            .read(gi)
            .read(shadow_maps)
            .read(vsm_shadow)
            .write(hdr);
    });
    // The MSAA resolve and the particles draw into the HDR target (`hdr_view`).
    g.add_pass("msaa_resolve_particles", |p| {
        p.read(depth).write(hdr);
    });
    // Additive light scatter into HDR (LoadOp::Load on `hdr_view`).
    g.add_pass("scatter_light", |p| {
        p.read(depth).read(clusters).read_write(hdr);
    });
    // TAA, bloom and tonemap read HDR; the velocity pass reads the depth and TAA reads last
    // frame's depth (`prev_depth`). The velocity, LDR and SMAA steps are intermediates of this
    // pass (written and read inside it, hence read_write).
    g.add_pass("taa_tonemap_bloom_smaa", |p| {
        p.read(hdr)
            .read(depth)
            .read(prev_depth)
            .read_write(taa_velocity)
            .read_write(ldr)
            .read_write(smaa_out)
            .read_write(taa_history)
            .write(backbuffer);
    });
    // Copies this frame's depth to `prev_depth` for the next frame's TAA test. The copy target
    // is an import, so the pass is kept by its write.
    g.add_pass("frame_tail", |p| {
        p.read(depth).write(prev_depth).side_effects();
    });
    g
}

/// Human-readable report of the compiled frame for the given target size (passes, resources,
/// slots, culled passes and access changes). See [`crate::render_graph::CompiledGraph::report`].
pub fn frame_graph_report(width: u32, height: u32) -> String {
    frame_graph(width, height)
        .compile()
        .expect("the frame graph is valid (checked by tests)")
        .report()
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

    /// The renderer's frame code: the dispatcher in `frame.rs` plus every `frame/*.rs` pass module.
    const FRAME_SOURCE: &str = concat!(
        include_str!("renderer/frame.rs"),
        include_str!("renderer/frame/post.rs"),
        include_str!("renderer/frame/shadows.rs"),
        include_str!("renderer/frame/vg.rs"),
    );

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
        // Transients: vg_clusters, depth, normals, hiz, ao, hdr_color (the other frame targets are
        // imports). They overlap in time or differ in format, so today the plan needs one slot
        // each. A future format merge would lower this number.
        let transients = 6;
        assert!(compiled.slot_count <= transients);
        assert_eq!(
            compiled.physical.iter().flatten().count(),
            transients,
            "every transient gets a slot"
        );
    }

    #[test]
    fn the_frame_graph_declares_valid_usage_and_no_self_dependency() {
        // compile() checks usage flags, read-before-write and self-dependencies; this also
        // covers the in-place history update of the TAA pass.
        assert!(frame_graph(1280, 720).compile().is_ok());
        assert!(frame_graph_report(1280, 720).contains("taa_tonemap_bloom_smaa"));
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
