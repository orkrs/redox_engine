//! MAT-03 node set: every node is checked against its expected value through the CPU
//! interpreter (`eval`), plus the noise and texture reference properties.

#![allow(clippy::unwrap_used)]

use redox_matgraph::noise::{self, NoiseKind};
use redox_matgraph::texture;
use redox_matgraph::{
    EvalEnv, EvalInputs, FlatTexture, MaterialGraph, Node, NodeKind, PortRef, SurfaceValues,
    TextureSampler, eval_with, lower, try_eval,
};

fn link(node: u32) -> Option<PortRef> {
    Some(PortRef { node, port: 0 })
}

fn node(id: u32, kind: NodeKind, inputs: Vec<Option<PortRef>>) -> Node {
    Node { id, kind, inputs }
}

fn constf(id: u32, v: f32) -> Node {
    node(id, NodeKind::ConstF32(v), vec![])
}

/// Node with one input linked to `a`.
fn un(id: u32, kind: NodeKind, a: u32) -> Node {
    node(id, kind, vec![link(a)])
}

fn bin(id: u32, kind: NodeKind, a: u32, b: u32) -> Node {
    node(id, kind, vec![link(a), link(b)])
}

fn inputs() -> EvalInputs {
    EvalInputs {
        uv: [0.25, 0.75],
        world_pos: [1.0, 2.0, 3.0],
        normal: [0.0, 1.0, 0.0],
        view_dir: [0.0, 0.0, 1.0],
        vertex_color: [1.0; 4],
        time: 0.5,
        params: Vec::new(),
    }
}

/// Graph whose surface output port `port` reads node `result`.
fn finish(mut nodes: Vec<Node>, result: u32, port: usize) -> MaterialGraph {
    let mut links = vec![None; 6];
    links[port] = link(result);
    nodes.push(node(99, NodeKind::SurfaceOutput, links));
    MaterialGraph {
        version: 1,
        name: "test".into(),
        nodes,
    }
}

/// Evaluates `graph` with `sampler` and optional tangent; asserts that it validates.
fn run(
    graph: &MaterialGraph,
    sampler: &dyn TextureSampler,
    tangent: Option<[f32; 3]>,
) -> SurfaceValues {
    let errors = graph.validate();
    assert!(errors.is_empty(), "{errors:?}");
    let ir = lower(graph).unwrap();
    eval_with(
        &ir,
        &inputs(),
        &EvalEnv {
            textures: sampler,
            tangent,
        },
    )
}

/// Opacity of a scalar node `result`.
fn scalar(nodes: Vec<Node>, result: u32) -> f32 {
    run(&finish(nodes, result, 1), &FlatTexture, None).opacity
}

/// Base colour (a `vec3`) of node `result`.
fn vector(nodes: Vec<Node>, result: u32) -> [f32; 3] {
    run(&finish(nodes, result, 0), &FlatTexture, None).base_color
}

fn unary(kind: NodeKind, x: f32) -> f32 {
    scalar(vec![constf(1, x), un(2, kind, 1)], 2)
}

fn binary(kind: NodeKind, a: f32, b: f32) -> f32 {
    scalar(vec![constf(1, a), constf(2, b), bin(3, kind, 1, 2)], 3)
}

fn ternary(kind: NodeKind, a: f32, b: f32, c: f32) -> f32 {
    scalar(
        vec![
            constf(1, a),
            constf(2, b),
            constf(3, c),
            node(4, kind, vec![link(1), link(2), link(3)]),
        ],
        4,
    )
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}

/// Sampler returning one texel for every read.
struct Solid([f32; 4]);

impl TextureSampler for Solid {
    fn sample(&self, _slot: u8, _uv: [f32; 2]) -> [f32; 4] {
        self.0
    }
}

// ---- math ----

#[test]
fn abs_removes_the_sign() {
    close(unary(NodeKind::Abs, -0.75), 0.75);
}

#[test]
fn floor_and_fract_split_a_value() {
    close(unary(NodeKind::Floor, 1.7), 1.0);
    close(unary(NodeKind::Fract, 1.25), 0.25);
}

#[test]
fn saturate_clamps_to_zero_one() {
    close(unary(NodeKind::Saturate, 1.5), 1.0);
    close(unary(NodeKind::Saturate, -0.5), 0.0);
}

#[test]
fn sin_cos_and_sqrt_match_std() {
    close(unary(NodeKind::Sin, 0.0), 0.0);
    close(unary(NodeKind::Cos, 0.0), 1.0);
    close(unary(NodeKind::Sqrt, 4.0), 2.0);
}

#[test]
fn remap_maps_the_input_range_onto_the_output_range() {
    let kind = NodeKind::Remap {
        in_min: 0.0,
        in_max: 2.0,
        out_min: 10.0,
        out_max: 20.0,
    };
    close(unary(kind.clone(), 1.0), 15.0);
    close(unary(kind, 0.0), 10.0);
}

#[test]
fn min_and_max_pick_the_extremes() {
    close(binary(NodeKind::Min, 3.0, 2.0), 2.0);
    close(binary(NodeKind::Max, 3.0, 2.0), 3.0);
}

#[test]
fn step_is_zero_below_the_edge_and_one_above() {
    close(binary(NodeKind::Step, 0.5, 0.3), 0.0);
    close(binary(NodeKind::Step, 0.5, 0.7), 1.0);
}

#[test]
fn smoothstep_clamps_outside_the_range() {
    close(ternary(NodeKind::Smoothstep, 0.0, 1.0, 0.5), 0.5);
    close(ternary(NodeKind::Smoothstep, 0.0, 1.0, 2.0), 1.0);
    close(ternary(NodeKind::Smoothstep, 0.0, 1.0, -1.0), 0.0);
}

#[test]
fn fresnel_is_zero_facing_and_one_at_grazing() {
    let kind = NodeKind::Fresnel { power: 5.0 };
    let facing = scalar(
        vec![
            node(1, NodeKind::ConstVec3([0.0, 1.0, 0.0]), vec![]),
            node(2, NodeKind::ConstVec3([0.0, 1.0, 0.0]), vec![]),
            bin(3, kind.clone(), 1, 2),
        ],
        3,
    );
    let grazing = scalar(
        vec![
            node(1, NodeKind::ConstVec3([1.0, 0.0, 0.0]), vec![]),
            node(2, NodeKind::ConstVec3([0.0, 1.0, 0.0]), vec![]),
            bin(3, kind, 1, 2),
        ],
        3,
    );
    close(facing, 0.0);
    close(grazing, 1.0);
}

#[test]
fn split_reads_one_component() {
    let v = scalar(
        vec![
            node(1, NodeKind::ConstVec3([1.0, 2.0, 3.0]), vec![]),
            un(2, NodeKind::Split { component: 2 }, 1),
        ],
        2,
    );
    close(v, 3.0);
}

#[test]
fn merge_builds_a_vector_from_three_scalars() {
    let v = vector(
        vec![
            constf(1, 1.0),
            constf(2, 2.0),
            constf(3, 3.0),
            node(4, NodeKind::Merge, vec![link(1), link(2), link(3)]),
        ],
        4,
    );
    assert_eq!(v, [1.0, 2.0, 3.0]);
}

#[test]
fn if_selects_by_the_mask() {
    close(ternary(NodeKind::If, 1.0, 4.0, 9.0), 4.0);
    close(ternary(NodeKind::If, 0.0, 4.0, 9.0), 9.0);
}

// ---- noise ----

#[test]
fn value_noise_stays_in_zero_one() {
    for i in 0..10_000u32 {
        let p = [i as f32 * 0.0137, i as f32 * 0.0291, 0.0];
        let v = noise::value(p, 2);
        assert!((0.0..=1.0 + 1e-6).contains(&v), "{p:?} -> {v}");
    }
}

#[test]
fn noise_is_deterministic_and_continuous() {
    for i in 0..200u32 {
        let p = [i as f32 * 0.173, i as f32 * 0.061, i as f32 * 0.029];
        for kind in [NoiseKind::Value, NoiseKind::Gradient] {
            let a = noise::eval(kind, p, 3);
            assert_eq!(a, noise::eval(kind, p, 3));
            let b = noise::eval(kind, [p[0] + 1e-3, p[1], p[2]], 3);
            assert!((a - b).abs() < 0.05, "{kind:?} at {p:?}: {a} vs {b}");
        }
    }
}

#[test]
fn worley_is_a_non_negative_distance() {
    for i in 0..500u32 {
        let p = [i as f32 * 0.19, i as f32 * 0.07, 0.0];
        assert!(noise::eval(NoiseKind::Worley, p, 2) >= 0.0);
    }
}

#[test]
fn each_noise_node_matches_the_reference_at_a_uv_point() {
    for (kind, node_kind) in [
        (NoiseKind::Value, NodeKind::ValueNoise),
        (NoiseKind::Gradient, NodeKind::GradientNoise),
        (NoiseKind::Worley, NodeKind::Worley),
    ] {
        let v = scalar(
            vec![node(1, NodeKind::Uv, vec![]), un(2, node_kind, 1)],
            2,
        );
        close(v, noise::eval(kind, [0.25, 0.75, 0.0], 2));
    }
}

// ---- textures ----

#[test]
fn texture2d_node_reads_the_sampler_texel() {
    let nodes = vec![
        node(1, NodeKind::Uv, vec![]),
        un(2, NodeKind::Texture2d { slot: 0 }, 1),
        un(3, NodeKind::Split { component: 0 }, 2),
    ];
    let v = run(&finish(nodes, 3, 1), &Solid([0.2, 0.4, 0.6, 1.0]), None).opacity;
    close(v, 0.2);
}

#[test]
fn texture_sample_node_reads_the_sampler_texel() {
    let nodes = vec![
        node(1, NodeKind::Uv, vec![]),
        un(2, NodeKind::TextureSample { slot: 0, clamp: true }, 1),
        un(3, NodeKind::Split { component: 1 }, 2),
    ];
    let v = run(&finish(nodes, 3, 1), &Solid([0.2, 0.4, 0.6, 1.0]), None).opacity;
    close(v, 0.4);
}

#[test]
fn a_flat_normal_texel_gives_the_surface_normal() {
    let nodes = vec![
        node(1, NodeKind::Uv, vec![]),
        un(2, NodeKind::NormalMap { slot: 0 }, 1),
    ];
    let out = run(&finish(nodes, 2, 2), &FlatTexture, None);
    close(out.normal[0], 0.0);
    close(out.normal[1], 1.0);
    close(out.normal[2], 0.0);
}

#[test]
fn a_tilted_normal_texel_rotates_into_the_tangent_frame() {
    // Texel (1, 0.5): decoded (1, 0, 0) in tangent space, so the result is the tangent.
    let nodes = vec![
        node(1, NodeKind::Uv, vec![]),
        un(2, NodeKind::NormalMap { slot: 0 }, 1),
    ];
    let out = run(
        &finish(nodes, 2, 2),
        &Solid([1.0, 0.5, 0.0, 1.0]),
        Some([1.0, 0.0, 0.0]),
    );
    close(out.normal[0], 1.0);
    close(out.normal[1], 0.0);
    close(out.normal[2], 0.0);
}

#[test]
fn triplanar_weights_sum_to_one() {
    let w = texture::triplanar_weights([0.6, 0.8, 0.0]);
    assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6, "{w:?}");
    assert!(w[1] > w[0], "the larger normal component dominates");
}

#[test]
fn triplanar_of_a_constant_texture_is_that_texel() {
    let nodes = vec![
        node(1, NodeKind::WorldPos, vec![]),
        node(2, NodeKind::Normal, vec![]),
        bin(3, NodeKind::Triplanar { slot: 0, scale: 1.0 }, 1, 2),
        un(4, NodeKind::Split { component: 0 }, 3),
    ];
    let v = run(&finish(nodes, 4, 1), &Solid([0.2, 0.4, 0.6, 1.0]), None).opacity;
    close(v, 0.2);
}

// ---- custom ----

#[test]
fn a_custom_node_has_no_cpu_reference() {
    let nodes = vec![
        constf(1, 1.5),
        node(
            2,
            NodeKind::Custom {
                name: "twice".into(),
                inputs: vec![("a".into(), redox_matgraph::Ty::F32)],
                output: redox_matgraph::Ty::F32,
                body: "    return a * 2.0;".into(),
            },
            vec![link(1)],
        ),
    ];
    let graph = finish(nodes, 2, 1);
    assert!(graph.validate().is_empty());
    let ir = lower(&graph).unwrap();
    assert!(try_eval(&ir, &inputs()).is_none());
}
