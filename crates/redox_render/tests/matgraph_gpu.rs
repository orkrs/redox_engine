//! Material graphs on the GPU (MAT-02 steps 4 and 5).
//!
//! Step 4 runs the generated `material_surface_graph` in a compute shader and compares it with
//! the CPU interpreter. Step 5 checks the shader cache. GPU tests are skipped without an adapter.

#![allow(clippy::unwrap_used)]

mod common;

use redox_matgraph::{
    CodegenOptions, EvalInputs, Features, MaterialGraph, Node, NodeKind, Pass, PortRef,
    SurfaceValues, Ty, compile_variant, eval, generate, lower, optimize,
};
use redox_render::matgraph_cache::{MaterialShaderCache, validate_wgsl, variant_source};
use redox_render::shader::create_shader_module;

/// Tolerance of the GPU result against the interpreter.
const TOLERANCE: f32 = 1e-4;

/// Bytes read back: six surface values, 12 floats.
const OUT_BYTES: u64 = 64;

/// Struct declarations the generated code needs. The full `material_surface.wgsl` is not used
/// because its derivative and discard code is not valid in a compute shader.
const HEADER: &str = "
struct SurfaceInput {
    world_pos: vec3<f32>,
    world_normal: vec3<f32>,
    world_tangent: vec3<f32>,
    instance_color: vec4<f32>,
    to_camera: vec3<f32>,
};

struct SurfaceData {
    base_color: vec3<f32>,
    opacity: f32,
    normal: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
    layer0: vec4<f32>,
    layer0_params: vec4<f32>,
    layer1: vec4<f32>,
    layer1_params: vec4<f32>,
    sheen: f32,
};
";

/// Entry point: the same fragment inputs as `EvalInputs` below, written as six floats each.
const COMPUTE: &str = "
@group(0) @binding(0) var<uniform> mat_params: MaterialParams;
@group(0) @binding(1) var<storage, read_write> out_values: array<f32>;

@compute @workgroup_size(1)
fn main() {
    let surface_in = SurfaceInput(
        vec3<f32>(0.0, 0.0, 0.0),
        vec3<f32>(0.0, 0.0, 1.0),
        vec3<f32>(1.0, 0.0, 0.0),
        vec4<f32>(1.0, 1.0, 1.0, 1.0),
        vec3<f32>(0.0, 0.0, 1.0)
    );
    let s = material_surface_graph(surface_in, vec2<f32>(0.25, 0.75), 0.0, mat_params);
    out_values[0] = s.base_color.x;
    out_values[1] = s.base_color.y;
    out_values[2] = s.base_color.z;
    out_values[3] = s.opacity;
    out_values[4] = s.normal.x;
    out_values[5] = s.normal.y;
    out_values[6] = s.normal.z;
    out_values[7] = s.metallic;
    out_values[8] = s.roughness;
    out_values[9] = s.emissive.x;
    out_values[10] = s.emissive.y;
    out_values[11] = s.emissive.z;
}
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
        name: "gpu_test".into(),
        nodes,
    }
}

/// Surface output fed by `base` on port 0 and nothing else.
fn output_of(id: u32, base: u32) -> Node {
    node(
        id,
        NodeKind::SurfaceOutput,
        vec![link(base), None, None, None, None, None],
    )
}

/// `base_color` is a constant colour.
fn constant_graph() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::ConstVec3([0.2, 0.4, 0.6]), vec![]),
        output_of(2, 1),
    ])
}

/// `base_color = lerp(red, blue, dot(uv, uv))`.
fn lerp_graph() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::ConstVec3([1.0, 0.0, 0.0]), vec![]),
        node(2, NodeKind::ConstVec3([0.0, 0.0, 1.0]), vec![]),
        node(3, NodeKind::Dot, vec![link(4), link(4)]),
        node(4, NodeKind::Uv, vec![]),
        node(5, NodeKind::Lerp, vec![link(1), link(2), link(3)]),
        output_of(6, 5),
    ])
}

/// `base_color` is a parameter `tint` with the given default.
fn param_graph(default: [f32; 4]) -> MaterialGraph {
    graph(vec![
        node(
            1,
            NodeKind::Param {
                name: "tint".into(),
                ty: Ty::Vec3,
                default,
            },
            vec![],
        ),
        output_of(2, 1),
    ])
}

/// `base_color = normalize([3, 0, 4])`, `opacity = pow(0.5, 2)`, `metallic = dot(normal, z)`.
fn math_graph() -> MaterialGraph {
    graph(vec![
        node(1, NodeKind::ConstVec3([3.0, 0.0, 4.0]), vec![]),
        node(2, NodeKind::Normalize, vec![link(1)]),
        node(3, NodeKind::ConstF32(0.5), vec![]),
        node(4, NodeKind::ConstF32(2.0), vec![]),
        node(5, NodeKind::Pow, vec![link(3), link(4)]),
        node(6, NodeKind::Normal, vec![]),
        node(7, NodeKind::ConstVec3([0.0, 0.0, 1.0]), vec![]),
        node(8, NodeKind::Dot, vec![link(6), link(7)]),
        node(
            9,
            NodeKind::SurfaceOutput,
            vec![link(2), link(5), None, link(8), None, None],
        ),
    ])
}

fn main_options() -> CodegenOptions {
    CodegenOptions {
        pass: Pass::Main,
        features: Features::default(),
    }
}

/// Fragment inputs shared by the GPU and the interpreter.
fn inputs(params: Vec<[f32; 4]>) -> EvalInputs {
    EvalInputs {
        uv: [0.25, 0.75],
        world_pos: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
        view_dir: [0.0, 0.0, 1.0],
        vertex_color: [1.0; 4],
        time: 0.0,
        params,
    }
}

/// Expected GPU output order: base (3), opacity, normal (3), metallic, roughness, emissive (3).
fn expected_floats(values: &SurfaceValues) -> Vec<f32> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&values.base_color);
    out.push(values.opacity);
    out.extend_from_slice(&values.normal);
    out.push(values.metallic);
    out.push(values.roughness);
    out.extend_from_slice(&values.emissive);
    out
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = common::adapter()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: redox_render::renderer::required_device_limits(&adapter.limits()),
        ..Default::default()
    }))
    .ok()
}

/// Runs the generated function of `graph` on the GPU and returns the 12 output floats.
fn run_on_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    graph: &MaterialGraph,
    params: &[[f32; 4]],
) -> Vec<f32> {
    let ir = optimize(lower(graph).unwrap());
    let generated = generate(&ir, &main_options());
    let source = variant_source(HEADER, &generated);
    let source = format!("{source}\n{COMPUTE}");
    validate_wgsl(&source).unwrap();

    let module = create_shader_module(device, "matgraph_eval", &source);
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("matgraph_eval"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let layout = pipeline.get_bind_group_layout(0);

    // One vec4 per slot; missing values fall back to the slot default, as in `eval`.
    let slots: Vec<[f32; 4]> = ir
        .params
        .iter()
        .enumerate()
        .map(|(i, slot)| params.get(i).copied().unwrap_or(slot.default))
        .collect();
    let mut bytes: Vec<u8> = bytemuck::cast_slice(&slots).to_vec();
    if bytes.is_empty() {
        // The generated struct has one padding member when there are no parameters.
        bytes = vec![0; 16];
    }
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("matgraph_params"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, &bytes);
    let storage = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("matgraph_out"),
        size: OUT_BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("matgraph_readback"),
        size: OUT_BYTES,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("matgraph_bind"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: storage.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("matgraph_eval"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("matgraph_eval"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&storage, 0, &readback, 0, OUT_BYTES);
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver.recv().unwrap().unwrap();
    let floats = {
        let data = slice.get_mapped_range().expect("mapped range");
        bytemuck::cast_slice::<u8, f32>(&data[..OUT_BYTES as usize]).to_vec()
    };
    readback.unmap();
    floats
}

/// Checks the GPU result against the interpreter within `TOLERANCE`.
fn assert_matches_interpreter(graph: &MaterialGraph, params: Vec<[f32; 4]>) {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let ir = optimize(lower(graph).unwrap());
    let expected = expected_floats(&eval(&ir, &inputs(params.clone())));
    let actual = run_on_gpu(&device, &queue, graph, &params);
    for (index, (got, want)) in actual.iter().zip(&expected).enumerate() {
        assert!(
            (got - want).abs() <= TOLERANCE,
            "component {index}: gpu {got} vs interpreter {want}"
        );
    }
}

#[test]
fn a_constant_colour_matches_the_interpreter() {
    assert_matches_interpreter(&constant_graph(), vec![]);
}

#[test]
fn a_lerp_by_uv_matches_the_interpreter() {
    assert_matches_interpreter(&lerp_graph(), vec![]);
}

#[test]
fn a_parameter_takes_its_value_from_the_uniform() {
    let graph = param_graph([0.1, 0.9, 0.2, 0.0]);
    assert_matches_interpreter(&graph, vec![[0.9, 0.1, 0.3, 0.0]]);
}

#[test]
fn normalize_pow_and_dot_match_the_interpreter() {
    let graph = math_graph();
    assert_matches_interpreter(&graph, vec![]);
    let Some((device, queue)) = device() else {
        return;
    };
    let gpu = run_on_gpu(&device, &queue, &graph, &[]);
    assert!((gpu[0] - 0.6).abs() <= TOLERANCE, "normalize x: {}", gpu[0]);
    assert!((gpu[2] - 0.8).abs() <= TOLERANCE, "normalize z: {}", gpu[2]);
    assert!((gpu[3] - 0.25).abs() <= TOLERANCE, "pow: {}", gpu[3]);
    assert!((gpu[7] - 1.0).abs() <= TOLERANCE, "dot: {}", gpu[7]);
}

#[test]
fn variants_are_cached() {
    let Some((device, _queue)) = device() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let graph = constant_graph();
    let first = compile_variant(&graph, &main_options()).unwrap();
    let second = compile_variant(&graph, &main_options()).unwrap();
    assert_eq!(first.key, second.key);

    let mut cache = MaterialShaderCache::new();
    let mut builds = 0;
    for _ in 0..2 {
        cache
            .get_or_build(&device, first.key, "variant", || {
                builds += 1;
                variant_source(HEADER, &first.generated)
            })
            .unwrap();
    }
    assert_eq!(builds, 1, "the second request reuses the module");
    assert_eq!(cache.len(), 1);
}

#[test]
fn two_instances_share_one_shader_with_different_params() {
    // Parameter values are uniform data, so two graphs that differ only in a default value
    // generate the same WGSL and therefore the same key and one shader module.
    let Some((device, _queue)) = device() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let red = compile_variant(&param_graph([1.0, 0.0, 0.0, 0.0]), &main_options()).unwrap();
    let blue = compile_variant(&param_graph([0.0, 0.0, 1.0, 0.0]), &main_options()).unwrap();
    assert_eq!(red.key, blue.key);

    let mut cache = MaterialShaderCache::new();
    cache
        .get_or_build(&device, red.key, "tint", || {
            variant_source(HEADER, &red.generated)
        })
        .unwrap();
    cache
        .get_or_build(&device, blue.key, "tint", || {
            variant_source(HEADER, &blue.generated)
        })
        .unwrap();
    assert_eq!(cache.len(), 1);
}

#[test]
fn a_broken_graph_keeps_the_previous_shader() {
    let Some((device, _queue)) = device() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let good = compile_variant(&constant_graph(), &main_options()).unwrap();
    let mut cache = MaterialShaderCache::new();
    cache
        .get_or_build(&device, good.key, "good", || {
            variant_source(HEADER, &good.generated)
        })
        .unwrap();

    let broken_key = redox_matgraph::ShaderKey(good.key.0.wrapping_add(1));
    let result = cache.get_or_build(&device, broken_key, "broken", || "fn broken( {".to_string());
    assert!(result.is_err(), "naga must refuse the broken source");
    assert_eq!(cache.len(), 1);
    assert!(cache.contains(good.key));
    assert!(!cache.contains(broken_key));
}

/// Graph that routes the scalar result of `op` to `roughness` (port 4).
fn scalar_case(mut nodes: Vec<Node>, op: u32) -> MaterialGraph {
    nodes.push(node(
        90,
        NodeKind::SurfaceOutput,
        vec![None, None, None, None, link(op), None],
    ));
    graph(nodes)
}

/// Graph that routes the vector result of `op` to `base_color` (port 0).
fn vector_case(mut nodes: Vec<Node>, op: u32) -> MaterialGraph {
    nodes.push(node(
        90,
        NodeKind::SurfaceOutput,
        vec![link(op), None, None, None, None, None],
    ));
    graph(nodes)
}

fn constant(id: u32, value: f32) -> Node {
    node(id, NodeKind::ConstF32(value), vec![])
}

/// Checks every graph of `cases` against the interpreter.
fn assert_all_match(cases: &[MaterialGraph]) {
    for case in cases {
        assert_matches_interpreter(case, vec![]);
    }
}

#[test]
fn unary_scalar_nodes_match_the_interpreter() {
    let kinds = [
        NodeKind::Abs,
        NodeKind::Floor,
        NodeKind::Fract,
        NodeKind::Saturate,
        NodeKind::Sin,
        NodeKind::Cos,
        NodeKind::Sqrt,
        NodeKind::OneMinus,
        NodeKind::Remap {
            in_min: 0.0,
            in_max: 1.0,
            out_min: -1.0,
            out_max: 1.0,
        },
    ];
    let cases: Vec<MaterialGraph> = kinds
        .into_iter()
        .map(|kind| {
            scalar_case(
                vec![constant(1, 2.3), node(2, kind, vec![link(1)])],
                2,
            )
        })
        .collect();
    assert_all_match(&cases);
}

#[test]
fn binary_and_ternary_scalar_nodes_match_the_interpreter() {
    let binary = [
        NodeKind::Add,
        NodeKind::Sub,
        NodeKind::Mul,
        NodeKind::Pow,
        NodeKind::Min,
        NodeKind::Max,
        NodeKind::Step,
    ];
    let mut cases: Vec<MaterialGraph> = binary
        .into_iter()
        .map(|kind| {
            scalar_case(
                vec![
                    constant(1, 0.5),
                    constant(2, 0.75),
                    node(3, kind, vec![link(1), link(2)]),
                ],
                3,
            )
        })
        .collect();
    let ternary = [NodeKind::Lerp, NodeKind::Clamp, NodeKind::Smoothstep];
    cases.extend(ternary.into_iter().map(|kind| {
        scalar_case(
            vec![
                constant(1, 0.2),
                constant(2, 0.9),
                constant(3, 0.6),
                node(4, kind, vec![link(1), link(2), link(3)]),
            ],
            4,
        )
    }));
    // select: mask 0.7 >= 0.5 picks input 1.
    cases.push(scalar_case(
        vec![
            constant(1, 0.7),
            constant(2, 0.3),
            constant(3, 0.8),
            node(4, NodeKind::If, vec![link(1), link(2), link(3)]),
        ],
        4,
    ));
    assert_all_match(&cases);
}

#[test]
fn vector_and_derived_nodes_match_the_interpreter() {
    let cases = [
        // Dot of uv with itself.
        scalar_case(
            vec![node(1, NodeKind::Uv, vec![]), node(2, NodeKind::Dot, vec![link(1), link(1)])],
            2,
        ),
        // Normalised constant vector.
        vector_case(
            vec![
                node(1, NodeKind::ConstVec3([1.0, 2.0, 2.0]), vec![]),
                node(2, NodeKind::Normalize, vec![link(1)]),
            ],
            2,
        ),
        // Component of uv.
        scalar_case(
            vec![node(1, NodeKind::Uv, vec![]), node(2, NodeKind::Split { component: 1 }, vec![link(1)])],
            2,
        ),
        // Component of vertex colour.
        scalar_case(
            vec![
                node(1, NodeKind::VertexColor, vec![]),
                node(2, NodeKind::Split { component: 3 }, vec![link(1)]),
            ],
            2,
        ),
        // Three scalars merged into a colour.
        vector_case(
            vec![
                constant(1, 0.1),
                constant(2, 0.5),
                constant(3, 0.9),
                node(4, NodeKind::Merge, vec![link(1), link(2), link(3)]),
            ],
            4,
        ),
        // Vector mix with a scalar factor from uv.
        vector_case(
            vec![
                node(1, NodeKind::ConstVec3([0.0, 1.0, 0.0]), vec![]),
                node(2, NodeKind::ConstVec3([1.0, 0.0, 1.0]), vec![]),
                node(3, NodeKind::Uv, vec![]),
                node(4, NodeKind::Split { component: 0 }, vec![link(3)]),
                node(5, NodeKind::Lerp, vec![link(1), link(2), link(4)]),
            ],
            5,
        ),
        // Fresnel of the normal against the view direction.
        scalar_case(
            vec![
                node(1, NodeKind::Normal, vec![]),
                node(2, NodeKind::ViewDir, vec![]),
                node(3, NodeKind::Fresnel { power: 2.0 }, vec![link(1), link(2)]),
            ],
            3,
        ),
        // Time input.
        scalar_case(vec![node(1, NodeKind::Time, vec![])], 1),
    ];
    assert_all_match(&cases);
}

#[test]
fn noise_nodes_match_the_interpreter() {
    let cases = [
        scalar_case(
            vec![node(1, NodeKind::Uv, vec![]), node(2, NodeKind::ValueNoise, vec![link(1)])],
            2,
        ),
        scalar_case(
            vec![node(1, NodeKind::Uv, vec![]), node(2, NodeKind::GradientNoise, vec![link(1)])],
            2,
        ),
        scalar_case(
            vec![node(1, NodeKind::Uv, vec![]), node(2, NodeKind::Worley, vec![link(1)])],
            2,
        ),
        scalar_case(
            vec![
                node(1, NodeKind::WorldPos, vec![]),
                node(2, NodeKind::ConstVec3([0.3, 0.6, 0.9]), vec![]),
                node(3, NodeKind::Add, vec![link(1), link(2)]),
                node(4, NodeKind::GradientNoise, vec![link(3)]),
            ],
            4,
        ),
    ];
    assert_all_match(&cases);
}
