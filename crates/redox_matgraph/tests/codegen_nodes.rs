//! MAT-03 code generation: the generated WGSL of the new nodes passes naga, custom nodes are
//! checked with line numbers, and the texture budget warns.

#![allow(clippy::unwrap_used)]

use redox_matgraph::{
    CodegenOptions, Features, MaterialGraph, Node, NodeKind, Pass, PortRef, Ty, generate, lower,
    optimize,
};

const PRELUDE: &str = include_str!("../../redox_render/shaders/material_surface.wgsl");

/// Bindings the renderer declares for generated texture code (group 9 keeps clear of the
/// prelude's own bindings).
const TEXTURE_DECLS: &str = "\
@group(9) @binding(0) var tex0: texture_2d<f32>;
@group(9) @binding(1) var samp_default: sampler;
@group(9) @binding(2) var samp_repeat: sampler;
@group(9) @binding(3) var samp_clamp: sampler;
";

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

fn main_pass() -> CodegenOptions {
    CodegenOptions {
        pass: Pass::Main,
        features: Features::default(),
    }
}

/// Parses and validates `source` with naga; panics with the diagnostics on failure.
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

/// Prelude, the texture declarations, the uniform struct and the generated function.
fn full_source(g: &MaterialGraph, decls: &str) -> String {
    let ir = optimize(lower(g).unwrap());
    let generated = generate(&ir, &main_pass());
    format!(
        "{PRELUDE}\n{decls}\n{}\n{}\n",
        generated.uniform_wgsl, generated.wgsl
    )
}

/// Graph using every texture node kind; the surface reads them through Split and Add.
fn texture_graph() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::Uv, vec![]),
        node(2, NodeKind::Texture2d { slot: 0 }, vec![link(1)]),
        node(3, NodeKind::Split { component: 0 }, vec![link(2)]),
        node(
            4,
            NodeKind::TextureSample {
                slot: 0,
                clamp: true,
            },
            vec![link(1)],
        ),
        node(5, NodeKind::Split { component: 1 }, vec![link(4)]),
        node(6, NodeKind::Add, vec![link(3), link(5)]),
        node(7, NodeKind::NormalMap { slot: 0 }, vec![link(1)]),
        node(8, NodeKind::Normal, vec![]),
        node(9, NodeKind::WorldPos, vec![]),
        node(
            10,
            NodeKind::Triplanar {
                slot: 0,
                scale: 2.0,
            },
            vec![link(9), link(8)],
        ),
        node(11, NodeKind::Split { component: 2 }, vec![link(10)]),
        node(12, NodeKind::Add, vec![link(6), link(11)]),
        node(
            13,
            NodeKind::SurfaceOutput,
            vec![link(7), link(12), None, None, None, None],
        ),
    ])
}

#[test]
fn a_texture_graph_validates_and_generates() {
    let g = texture_graph();
    assert!(g.validate().is_empty(), "{:?}", g.validate());
    let source = full_source(&g, TEXTURE_DECLS);
    validate_wgsl(&source);
    assert!(
        source.contains("textureSample(tex0, samp_default"),
        "{source}"
    );
    assert!(
        source.contains("textureSample(tex0, samp_clamp"),
        "{source}"
    );
    assert!(source.contains("mg_normal_map(tex0"), "{source}");
    assert!(source.contains("mg_triplanar(tex0"), "{source}");
}

#[test]
fn a_noise_graph_validates_and_generates() {
    let g = graph(vec![
        node(1, NodeKind::Uv, vec![]),
        node(2, NodeKind::ValueNoise, vec![link(1)]),
        node(3, NodeKind::GradientNoise, vec![link(1)]),
        node(4, NodeKind::Worley, vec![link(1)]),
        node(5, NodeKind::WorldPos, vec![]),
        node(6, NodeKind::ValueNoise, vec![link(5)]),
        node(7, NodeKind::Add, vec![link(2), link(3)]),
        node(8, NodeKind::Add, vec![link(7), link(4)]),
        node(9, NodeKind::Add, vec![link(8), link(6)]),
        node(
            10,
            NodeKind::SurfaceOutput,
            vec![None, link(9), None, None, None, None],
        ),
    ]);
    assert!(g.validate().is_empty(), "{:?}", g.validate());
    let source = full_source(&g, "");
    validate_wgsl(&source);
    assert!(source.contains("mg_value_noise(vec3<f32>("), "{source}");
    assert!(source.contains("mg_gradient_noise("), "{source}");
    assert!(source.contains("mg_worley("), "{source}");
    assert!(source.contains("fn mg_pcg("), "helper block missing");
}

#[test]
fn math_and_logic_nodes_validate_and_generate() {
    let g = graph(vec![
        // Inputs that do not fold away, so the generated code keeps every operation.
        node(1, NodeKind::Time, vec![]),
        node(2, NodeKind::Abs, vec![link(1)]),
        node(3, NodeKind::Floor, vec![link(2)]),
        node(4, NodeKind::Fract, vec![link(2)]),
        node(5, NodeKind::Saturate, vec![link(4)]),
        node(6, NodeKind::Sin, vec![link(5)]),
        node(7, NodeKind::Cos, vec![link(6)]),
        node(8, NodeKind::Sqrt, vec![link(7)]),
        node(
            9,
            NodeKind::Remap {
                in_min: 0.0,
                in_max: 1.0,
                out_min: 0.0,
                out_max: 2.0,
            },
            vec![link(8)],
        ),
        node(10, NodeKind::Min, vec![link(9), link(1)]),
        node(11, NodeKind::Max, vec![link(10), link(2)]),
        node(12, NodeKind::Step, vec![link(1), link(11)]),
        node(13, NodeKind::Smoothstep, vec![link(1), link(11), link(9)]),
        node(14, NodeKind::ConstVec3([0.1, 0.2, 0.3]), vec![]),
        node(15, NodeKind::Split { component: 1 }, vec![link(14)]),
        node(16, NodeKind::Merge, vec![link(13), link(15), link(12)]),
        node(17, NodeKind::Normal, vec![]),
        node(18, NodeKind::ViewDir, vec![]),
        node(
            19,
            NodeKind::Fresnel { power: 3.0 },
            vec![link(17), link(18)],
        ),
        node(20, NodeKind::If, vec![link(19), link(13), link(12)]),
        node(
            21,
            NodeKind::SurfaceOutput,
            vec![link(16), link(20), None, None, None, None],
        ),
    ]);
    assert!(g.validate().is_empty(), "{:?}", g.validate());
    let source = full_source(&g, "");
    validate_wgsl(&source);
    assert!(source.contains("smoothstep("), "{source}");
    assert!(source.contains("select("), "{source}");
    assert!(source.contains("pow(1.0 - saturate(dot("), "{source}");
}

fn custom_graph(body: &str) -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::ConstF32(1.5), vec![]),
        node(
            2,
            NodeKind::Custom {
                name: "tweak".into(),
                inputs: vec![("a".into(), Ty::F32)],
                output: Ty::F32,
                body: body.into(),
            },
            vec![link(1)],
        ),
        node(
            3,
            NodeKind::SurfaceOutput,
            vec![None, link(2), None, None, None, None],
        ),
    ])
}

#[test]
fn a_valid_custom_node_generates_and_validates() {
    let g = custom_graph("    return a * 2.0;");
    assert!(g.validate().is_empty(), "{:?}", g.validate());
    let source = full_source(&g, "");
    validate_wgsl(&source);
    assert!(source.contains("fn custom_2(a: f32) -> f32 {"), "{source}");
}

#[test]
fn a_syntax_error_reports_the_line_inside_the_body() {
    let errors = custom_graph("    let t = a;\n    let u = t +;\n    return t;").validate();
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .any(|e| e.node == Some(2) && e.message.contains("строка 2")),
        "{errors:?}"
    );
}

#[test]
fn a_custom_node_cannot_declare_bindings() {
    let errors =
        custom_graph("    @group(0) @binding(0) var<uniform> x: f32;\n    return a;").validate();
    assert!(errors.iter().any(|e| e.node == Some(2)), "{errors:?}");
}

#[test]
fn a_custom_node_cannot_enable_extensions() {
    let errors = custom_graph("    enable f16;\n    return a;").validate();
    assert!(errors.iter().any(|e| e.node == Some(2)), "{errors:?}");
}

#[test]
fn a_body_that_closes_the_function_early_is_rejected() {
    let errors = custom_graph("    return a; }\nfn evil() -> f32 { return 0.0;").validate();
    assert!(errors.iter().any(|e| e.node == Some(2)), "{errors:?}");
}

#[test]
fn an_oversized_body_is_rejected() {
    let body = format!("{}    return a;", " ".repeat(5000));
    let errors = custom_graph(&body).validate();
    assert!(
        errors.iter().any(|e| e.message.contains("предел")),
        "{errors:?}"
    );
}

#[test]
fn a_texture_budget_above_sixteen_samples_warns() {
    let mut nodes = vec![node(1, NodeKind::Uv, vec![])];
    for id in 2..=18 {
        nodes.push(node(id, NodeKind::Texture2d { slot: 0 }, vec![link(1)]));
    }
    nodes.push(node(19, NodeKind::ConstVec3([0.0; 3]), vec![]));
    nodes.push(node(
        20,
        NodeKind::SurfaceOutput,
        vec![link(19), None, None, None, None, None],
    ));
    let g = graph(nodes);
    assert!(g.validate().is_empty(), "{:?}", g.validate());
    assert_eq!(g.warnings().len(), 1);
}
