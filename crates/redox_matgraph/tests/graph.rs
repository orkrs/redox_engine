//! Material graph (MAT-02): validation, RON round trip, lowering, optimisation, code generation.

#![allow(clippy::unwrap_used)]

use redox_matgraph::{
    CodegenOptions, EvalInputs, Features, MaterialGraph, Node, NodeKind, Pass, PortRef, Ty, eval,
    generate, lower, optimize, variant_key,
};

fn link(node: u32) -> Option<PortRef> {
    Some(PortRef { node, port: 0 })
}

fn node(id: u32, kind: NodeKind, inputs: Vec<Option<PortRef>>) -> Node {
    Node { id, kind, inputs }
}

fn graph(nodes: Vec<Node>) -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "test".into(),
        nodes,
    }
}

/// `base_color = lerp(red, blue, time)` -> output.
fn lerp_graph() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::ConstVec3([1.0, 0.0, 0.0]), vec![]),
        node(2, NodeKind::ConstVec3([0.0, 0.0, 1.0]), vec![]),
        node(3, NodeKind::Time, vec![]),
        node(4, NodeKind::Lerp, vec![link(1), link(2), link(3)]),
        node(
            5,
            NodeKind::SurfaceOutput,
            vec![link(4), None, None, None, None, None],
        ),
    ])
}

#[test]
fn a_valid_graph_has_no_errors() {
    let g = graph(vec![
        node(1, NodeKind::ConstVec3([1.0, 0.0, 0.0]), vec![]),
        node(2, NodeKind::ConstVec3([0.0, 0.0, 1.0]), vec![]),
        node(3, NodeKind::ConstF32(0.5), vec![]),
        node(4, NodeKind::Lerp, vec![link(1), link(2), link(3)]),
        node(
            5,
            NodeKind::SurfaceOutput,
            vec![link(4), None, None, None, None, None],
        ),
    ]);
    assert_eq!(g.validate(), vec![]);
}

#[test]
fn ron_roundtrip_is_lossless_and_deterministic() {
    let g = lerp_graph();
    let text = g.to_ron();
    let back = MaterialGraph::from_ron(&text).unwrap();
    let mut shuffled = g.clone();
    shuffled.nodes.reverse();
    assert_eq!(back.to_ron(), text);
    assert_eq!(shuffled.to_ron(), text, "node order does not matter");
}

#[test]
fn a_cycle_is_reported_with_its_nodes() {
    let g = graph(vec![
        node(1, NodeKind::Add, vec![link(2), link(2)]),
        node(2, NodeKind::Add, vec![link(1), link(1)]),
        node(
            3,
            NodeKind::SurfaceOutput,
            vec![None, link(1), None, None, None, None],
        ),
    ]);
    let errors = g.validate();
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains('1') && e.message.contains('2'))
    );
}

#[test]
fn a_type_mismatch_names_the_node() {
    // A Vec3 into the scalar opacity port.
    let g = graph(vec![
        node(1, NodeKind::ConstVec3([1.0, 0.0, 0.0]), vec![]),
        node(
            2,
            NodeKind::SurfaceOutput,
            vec![None, link(1), None, None, None, None],
        ),
    ]);
    let errors = g.validate();
    assert!(errors.iter().any(|e| e.node == Some(2)), "{errors:?}");
}

#[test]
fn a_missing_output_is_an_error() {
    let g = graph(vec![node(1, NodeKind::ConstF32(1.0), vec![])]);
    assert!(!g.validate().is_empty());
}

#[test]
fn two_outputs_are_an_error() {
    let out = |id| node(id, NodeKind::SurfaceOutput, vec![None; 6]);
    assert!(!graph(vec![out(1), out(2)]).validate().is_empty());
}

#[test]
fn a_link_to_a_missing_node_is_an_error() {
    let g = graph(vec![node(
        1,
        NodeKind::SurfaceOutput,
        vec![link(99), None, None, None, None, None],
    )]);
    assert!(!g.validate().is_empty());
}

#[test]
fn duplicate_param_names_are_an_error() {
    let param = |id| {
        node(
            id,
            NodeKind::Param {
                name: "tint".into(),
                ty: Ty::F32,
                default: [1.0; 4],
            },
            vec![],
        )
    };
    let g = graph(vec![
        param(1),
        param(2),
        node(
            3,
            NodeKind::SurfaceOutput,
            vec![None, link(1), None, None, link(2), None],
        ),
    ]);
    assert!(!g.validate().is_empty());
}

#[test]
fn scalar_broadcast_is_allowed_in_arithmetic() {
    let g = graph(vec![
        node(1, NodeKind::ConstVec3([1.0, 0.5, 0.25]), vec![]),
        node(2, NodeKind::ConstF32(2.0), vec![]),
        node(3, NodeKind::Mul, vec![link(1), link(2)]),
        node(
            4,
            NodeKind::SurfaceOutput,
            vec![link(3), None, None, None, None, None],
        ),
    ]);
    assert_eq!(g.validate(), vec![]);
}

#[test]
fn an_unknown_version_is_rejected() {
    let mut g = lerp_graph();
    g.version = 99;
    assert!(!g.validate().is_empty());
}

// ---- IR ----

fn constant_graph() -> MaterialGraph {
    // opacity = (1 + 2) * 0.5, base colour = default.
    graph(vec![
        node(1, NodeKind::ConstF32(1.0), vec![]),
        node(2, NodeKind::ConstF32(2.0), vec![]),
        node(3, NodeKind::Add, vec![link(1), link(2)]),
        node(4, NodeKind::ConstF32(0.5), vec![]),
        node(5, NodeKind::Mul, vec![link(3), link(4)]),
        node(
            6,
            NodeKind::SurfaceOutput,
            vec![None, link(5), None, None, None, None],
        ),
    ])
}

fn inputs() -> EvalInputs {
    EvalInputs {
        uv: [0.25, 0.75],
        world_pos: [0.0; 3],
        normal: [0.0, 1.0, 0.0],
        view_dir: [0.0, 0.0, 1.0],
        vertex_color: [1.0; 4],
        time: 0.0,
        params: Vec::new(),
    }
}

#[test]
fn lowering_orders_operations_so_arguments_come_first() {
    let ir = lower(&lerp_graph()).unwrap_or_else(|e| panic!("{e:?}"));
    for op in &ir.ops {
        assert!(op.args.iter().all(|&a| a < op.id), "{op:?}");
    }
}

#[test]
fn constants_are_folded() {
    let ir = lower(&constant_graph()).unwrap();
    let before = ir.ops.len();
    let after = optimize(ir.clone());
    assert!(after.ops.len() < before, "{before} -> {}", after.ops.len());
}

#[test]
fn optimisation_does_not_change_the_result_of_a_constant_graph() {
    let ir = lower(&constant_graph()).unwrap();
    let a = eval(&ir, &inputs());
    let b = eval(&optimize(ir), &inputs());
    assert!((a.opacity - 1.5).abs() < 1e-6);
    assert_eq!(a, b);
}

#[test]
fn dead_branches_are_removed() {
    let mut g = constant_graph();
    // A node with no path to the output.
    g.nodes.push(node(9, NodeKind::Time, vec![]));
    let ir = optimize(lower(&g).unwrap());
    assert!(ir.ops.iter().all(|op| format!("{:?}", op.kind) != "Time"));
}

#[test]
fn common_subexpressions_are_shared() {
    let g = graph(vec![
        node(1, NodeKind::Uv, vec![]),
        node(2, NodeKind::ConstF32(2.0), vec![]),
        node(3, NodeKind::Mul, vec![link(1), link(2)]),
        node(4, NodeKind::Mul, vec![link(1), link(2)]),
        node(5, NodeKind::Add, vec![link(3), link(4)]),
        node(
            6,
            NodeKind::SurfaceOutput,
            vec![None, None, None, None, None, None],
        ),
    ]);
    // The unused Add is dropped; build a graph that actually uses both and count the Mul ops.
    let _ = g;
    let used = graph(vec![
        node(1, NodeKind::Time, vec![]),
        node(2, NodeKind::ConstF32(2.0), vec![]),
        node(3, NodeKind::Mul, vec![link(1), link(2)]),
        node(4, NodeKind::Mul, vec![link(1), link(2)]),
        node(5, NodeKind::Add, vec![link(3), link(4)]),
        node(
            6,
            NodeKind::SurfaceOutput,
            vec![None, link(5), None, None, None, None],
        ),
    ]);
    let ir = optimize(lower(&used).unwrap());
    let muls = ir
        .ops
        .iter()
        .filter(|op| format!("{:?}", op.kind).starts_with("Mul"))
        .count();
    assert_eq!(muls, 1);
}

#[test]
fn parameters_get_stable_16_byte_slots_sorted_by_name() {
    let param = |id, name: &str| {
        node(
            id,
            NodeKind::Param {
                name: name.into(),
                ty: Ty::F32,
                default: [id as f32, 0.0, 0.0, 0.0],
            },
            vec![],
        )
    };
    let g = graph(vec![
        param(1, "zeta"),
        param(2, "alpha"),
        node(
            3,
            NodeKind::SurfaceOutput,
            vec![None, link(1), None, None, link(2), None],
        ),
    ]);
    let ir = lower(&g).unwrap();
    let names: Vec<&str> = ir.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "zeta"]);
    let offsets: Vec<u32> = ir.params.iter().map(|p| p.offset).collect();
    assert_eq!(offsets, vec![0, 4]);
}

// ---- code generation ----

const PRELUDE: &str = include_str!("../../redox_render/shaders/material_surface.wgsl");

fn validate_wgsl(source: &str) {
    let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| {
        panic!("{}\n{source}", e.emit_to_string(source));
    });
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{e:?}\n{source}"));
}

fn full_source(ir: &redox_matgraph::Ir, opts: &CodegenOptions) -> String {
    let g = generate(ir, opts);
    format!("{PRELUDE}\n{}\n{}\n", g.uniform_wgsl, g.wgsl)
}

fn main_pass() -> CodegenOptions {
    CodegenOptions {
        pass: Pass::Main,
        features: Features::default(),
    }
}

#[test]
fn the_generated_code_passes_naga_validation() {
    for g in [lerp_like(), constant_graph()] {
        let ir = optimize(lower(&g).unwrap());
        for pass in [Pass::Main, Pass::Depth, Pass::Shadow] {
            let opts = CodegenOptions {
                pass,
                features: Features::default(),
            };
            validate_wgsl(&full_source(&ir, &opts));
        }
    }
}

/// A valid mix of nodes: params, arithmetic, pow, normalize, dot.
fn lerp_like() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::ConstVec3([1.0, 0.0, 0.0]), vec![]),
        node(
            2,
            NodeKind::Param {
                name: "tint".into(),
                ty: Ty::Vec3,
                default: [0.0, 0.0, 1.0, 0.0],
            },
            vec![],
        ),
        node(3, NodeKind::Time, vec![]),
        node(4, NodeKind::Lerp, vec![link(1), link(2), link(3)]),
        node(5, NodeKind::Normal, vec![]),
        node(6, NodeKind::Normalize, vec![link(5)]),
        node(7, NodeKind::ViewDir, vec![]),
        node(8, NodeKind::Dot, vec![link(6), link(7)]),
        node(9, NodeKind::ConstF32(2.0), vec![]),
        node(10, NodeKind::Pow, vec![link(8), link(9)]),
        node(
            11,
            NodeKind::SurfaceOutput,
            vec![link(4), link(10), None, None, None, None],
        ),
    ])
}

#[test]
fn generation_is_deterministic() {
    let ir = optimize(lower(&lerp_like()).unwrap());
    assert_eq!(generate(&ir, &main_pass()), generate(&ir, &main_pass()));
}

#[test]
fn parameters_appear_in_the_uniform_struct_in_slot_order() {
    let ir = optimize(lower(&lerp_like()).unwrap());
    let g = generate(&ir, &main_pass());
    assert!(g.uniform_wgsl.contains("p_tint"), "{}", g.uniform_wgsl);
    assert_eq!(g.param_bytes, 16);
}

#[test]
fn a_depth_variant_ignores_properties_that_do_not_affect_opacity() {
    let ir = optimize(lower(&lerp_like()).unwrap());
    let main = generate(&ir, &main_pass()).wgsl;
    let depth = generate(
        &ir,
        &CodegenOptions {
            pass: Pass::Depth,
            features: Features::default(),
        },
    )
    .wgsl;
    assert!(depth.len() <= main.len());
    assert!(!depth.contains("p_tint"), "{depth}");
}

#[test]
fn the_variant_key_changes_with_the_pass_and_is_stable() {
    let ir = optimize(lower(&lerp_like()).unwrap());
    let a = variant_key(&ir, &main_pass());
    let b = variant_key(&ir, &main_pass());
    let c = variant_key(
        &ir,
        &CodegenOptions {
            pass: Pass::Shadow,
            features: Features::default(),
        },
    );
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn the_interpreter_blends_two_colours_by_a_scalar() {
    let g = graph(vec![
        node(1, NodeKind::ConstVec3([1.0, 0.0, 0.0]), vec![]),
        node(2, NodeKind::ConstVec3([0.0, 0.0, 1.0]), vec![]),
        node(3, NodeKind::ConstF32(0.25), vec![]),
        node(4, NodeKind::Lerp, vec![link(1), link(2), link(3)]),
        node(
            5,
            NodeKind::SurfaceOutput,
            vec![link(4), None, None, None, None, None],
        ),
    ]);
    let out = eval(&lower(&g).unwrap(), &inputs());
    assert!((out.base_color[0] - 0.75).abs() < 1e-6);
    assert!((out.base_color[2] - 0.25).abs() < 1e-6);
}
