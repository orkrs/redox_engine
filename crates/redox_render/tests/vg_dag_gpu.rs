//! GPU traversal of cluster DAGs (VG-02). Every test is skipped when no adapter is available.

#![allow(clippy::unwrap_used)]

mod common;

use bytemuck::Zeroable;
use redox_render::mesh::Mesh;
use redox_render::mesh::primitive::create_sphere;
use redox_render::virtual_geometry::dag::{
    ClusterDag, DagInstance, DagParams, build_cluster_dag, cut,
};
use redox_render::virtual_geometry::dag_gpu::{COUNTER_CMD, COUNTER_REJECT, DagFrame, DagTraversal};
use redox_render::virtual_geometry::gpu_cull::CullParamsUniform;
use redox_render::virtual_geometry::meshlet::DrawIndexedIndirectCmd;

const PPU: f32 = 1000.0;
const THRESHOLD_PX: f32 = 1.0;
const CMD_CAPACITY: usize = 1 << 16;
const DEFAULT_QUEUE: u32 = 1 << 16;

fn sphere() -> Mesh {
    create_sphere(1.0, 48)
}

fn build(mesh: &Mesh) -> ClusterDag {
    build_cluster_dag(&mesh.vertices, &mesh.indices, &DagParams::default())
        .unwrap()
        .0
}

fn at(x: f32) -> DagInstance {
    DagInstance {
        dag: 0,
        transform: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [x, 0.0, 0.0, 1.0],
        ],
    }
}

/// Result of one traversal read back from the GPU.
struct Outcome {
    /// Draw commands as `(first_index, index_count)`, sorted.
    draws: Vec<(u32, u32)>,
    /// `[queue A, queue B, draw commands, overflows]` counters after the frame.
    counters: Vec<u32>,
}

/// Runs one traversal of `dags` with `instances` seen from `eye` and reads the result back.
fn traverse(
    gpu: &(wgpu::Device, wgpu::Queue),
    dags: &[ClusterDag],
    instances: &[DagInstance],
    eye: [f32; 3],
    cull_frustum: bool,
    cull: &CullParamsUniform,
    queue_capacity: u32,
) -> Outcome {
    let (device, queue) = gpu;
    let mut traversal = DagTraversal::new(device, queue_capacity, instances.len() as u32, 1 << 16, 1 << 17);
    traversal
        .upload_dags(queue, dags, &vec![(0, 0); dags.len()])
        .unwrap();

    let hiz = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vg_dag_test_hiz"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let hiz_view = hiz.create_view(&wgpu::TextureViewDescriptor::default());
    let hiz_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());

    let commands = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vg_dag_test_commands"),
        size: (CMD_CAPACITY * std::mem::size_of::<DrawIndexedIndirectCmd>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let frame = DagFrame {
        instances,
        eye,
        pixels_per_unit: PPU,
        threshold_px: THRESHOLD_PX,
        cull_frustum,
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let passes = traversal.record(
        queue,
        device,
        &mut encoder,
        &frame,
        cull,
        (&hiz_view, &hiz_sampler),
        &commands,
    );
    assert!(passes > 0, "the hierarchy must be uploaded");
    queue.submit(Some(encoder.finish()));

    let counters_bytes = read_back(device, queue, traversal.counters_buffer(), 20);
    let counters: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&counters_bytes).to_vec();
    let count = (counters[2] as usize).min(CMD_CAPACITY);
    let cmd_bytes = read_back(
        device,
        queue,
        &commands,
        (CMD_CAPACITY * std::mem::size_of::<DrawIndexedIndirectCmd>()) as u64,
    );
    let cmds: &[DrawIndexedIndirectCmd] = bytemuck::cast_slice(&cmd_bytes);
    let mut draws: Vec<(u32, u32)> = cmds[..count]
        .iter()
        .map(|c| (c.first_index, c.index_count))
        .collect();
    draws.sort_unstable();
    Outcome { draws, counters }
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, src: &wgpu::Buffer, size: u64) -> Vec<u8> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vg_dag_test_staging"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_buffer_to_buffer(src, 0, &staging, 0, size);
    queue.submit(Some(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range().expect("buffer is mapped").to_vec();
    staging.unmap();
    data
}

/// Index ranges `(index_offset, index_count)` of the given cluster ids, sorted.
fn ranges_of(dag: &ClusterDag, clusters: &[u32]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = clusters
        .iter()
        .map(|&c| {
            let c = &dag.clusters[c as usize];
            (c.index_offset, c.index_count)
        })
        .collect();
    out.sort_unstable();
    out
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = common::adapter()?;
    common::device_with_bc(&adapter)
}

#[test]
fn the_gpu_selection_equals_the_cpu_cut_for_a_sphere() {
    let Some(gpu) = gpu() else { return };
    let dag = build(&sphere());
    let no_cull = CullParamsUniform::zeroed();
    for distance in [4.0f32, 8.0, 40.0] {
        let eye = [0.0, 0.0, distance];
        let outcome = traverse(
            &gpu,
            std::slice::from_ref(&dag),
            &[at(0.0)],
            eye,
            false,
            &no_cull,
            DEFAULT_QUEUE,
        );
        let want = ranges_of(&dag, &cut(&dag, eye, PPU, THRESHOLD_PX));
        assert_eq!(outcome.draws, want, "eye at {distance} m");
    }
}

#[test]
fn a_far_instance_selects_only_root_clusters() {
    let Some(gpu) = gpu() else { return };
    let dag = build(&sphere());
    let roots: Vec<u32> = (0..dag.clusters.len() as u32)
        .filter(|&i| dag.clusters[i as usize].parent_error == f32::INFINITY)
        .collect();
    let outcome = traverse(
        &gpu,
        std::slice::from_ref(&dag),
        &[at(0.0)],
        [0.0, 0.0, 1.0e6],
        false,
        &CullParamsUniform::zeroed(),
        DEFAULT_QUEUE,
    );
    assert_eq!(outcome.draws, ranges_of(&dag, &roots));
}

#[test]
fn a_near_instance_selects_leaves() {
    let Some(gpu) = gpu() else { return };
    let dag = build(&sphere());
    let eye = [0.0, 0.0, 1.001];
    let outcome = traverse(
        &gpu,
        std::slice::from_ref(&dag),
        &[at(0.0)],
        eye,
        false,
        &CullParamsUniform::zeroed(),
        DEFAULT_QUEUE,
    );
    let leaves: Vec<u32> = (0..dag.clusters.len() as u32)
        .filter(|&i| dag.clusters[i as usize].group == u32::MAX)
        .collect();
    assert_eq!(outcome.draws, ranges_of(&dag, &leaves));
}

#[test]
fn an_instance_outside_the_frustum_selects_nothing() {
    let Some(gpu) = gpu() else { return };
    let dag = build(&sphere());
    // Plane x >= 1e6 rejects every cluster near the origin; the other planes are pass-through.
    let mut cull = CullParamsUniform::zeroed();
    cull.planes[0] = [1.0, 0.0, 0.0, -1.0e6];
    let outcome = traverse(
        &gpu,
        std::slice::from_ref(&dag),
        &[at(0.0)],
        [0.0, 0.0, 6.0],
        true,
        &cull,
        DEFAULT_QUEUE,
    );
    assert!(outcome.draws.is_empty());
    assert_eq!(outcome.counters[2], 0);
}

#[test]
fn queue_overflow_coarsens_instead_of_dropping() {
    let Some(gpu) = gpu() else { return };
    let dag = build(&sphere());
    let outcome = traverse(
        &gpu,
        std::slice::from_ref(&dag),
        &[at(0.0)],
        [0.0, 0.0, 1.001],
        false,
        &CullParamsUniform::zeroed(),
        64,
    );
    assert!(outcome.counters[3] > 0, "a 64-entry queue must overflow");
    assert!(!outcome.draws.is_empty(), "overflow must draw coarser clusters");
}

/// 1x1 Hi-Z pyramid holding `value`; returns the texture too so that it can be rewritten later.
fn hiz_with(device: &wgpu::Device, queue: &wgpu::Queue, value: f32) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vg_dag_test_hiz"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_hiz(queue, &texture, value);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    (texture, view, sampler)
}

fn write_hiz(queue: &wgpu::Queue, texture: &wgpu::Texture, value: f32) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &value.to_le_bytes(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

#[test]
fn hiz_rejects_are_retested_after_the_pyramid_is_rebuilt() {
    let Some((device, queue)) = gpu() else { return };
    let dag = build(&sphere());
    let eye = [0.0, 0.0, 6.0];
    // Identity view-projection: clip space is world space, so the occlusion test compares the
    // sphere's nearest point with the pyramid value directly.
    let mut cull = CullParamsUniform::zeroed();
    cull.view_proj = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    cull.camera_pos = [eye[0], eye[1], eye[2], 1.0];
    cull.hiz_size = [1.0, 1.0];
    cull.hiz_mip_count = 1;
    cull.hiz_enabled = 1;

    // Previous-frame pyramid: far below every nearest point, so everything is occluded.
    let (texture, view, sampler) = hiz_with(&device, &queue, -10.0);
    let mut traversal = DagTraversal::new(&device, DEFAULT_QUEUE, 1, 1 << 16, 1 << 17);
    traversal.upload_dags(&queue, std::slice::from_ref(&dag), &[(0, 0)]).unwrap();
    let commands = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vg_dag_test_commands"),
        size: (CMD_CAPACITY * std::mem::size_of::<DrawIndexedIndirectCmd>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let instances = [at(0.0)];
    let frame = DagFrame {
        instances: &instances,
        eye,
        pixels_per_unit: PPU,
        threshold_px: THRESHOLD_PX,
        cull_frustum: false,
    };

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    traversal.record(&queue, &device, &mut encoder, &frame, &cull, (&view, &sampler), &commands);
    queue.submit(Some(encoder.finish()));
    let phase1 = read_back(&device, &queue, traversal.counters_buffer(), 20);
    let phase1: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&phase1).to_vec();
    assert!(phase1[COUNTER_REJECT] > 0, "occluded clusters must be recorded as rejects");
    assert_eq!(phase1[COUNTER_CMD], 0, "nothing is drawn before the retest");

    // Pyramid rebuilt for this frame: nothing occludes the sphere any more.
    write_hiz(&queue, &texture, 10.0);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    traversal.record_retest(&queue, &device, &mut encoder, &cull, (&view, &sampler), &commands);
    queue.submit(Some(encoder.finish()));

    let counters = read_back(&device, &queue, traversal.counters_buffer(), 20);
    let counters: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&counters).to_vec();
    let cmds = read_back(
        &device,
        &queue,
        &commands,
        (CMD_CAPACITY * std::mem::size_of::<DrawIndexedIndirectCmd>()) as u64,
    );
    let cmds: &[DrawIndexedIndirectCmd] = bytemuck::cast_slice(&cmds);
    let mut draws: Vec<(u32, u32)> = cmds[..(counters[COUNTER_CMD] as usize).min(CMD_CAPACITY)]
        .iter()
        .map(|c| (c.first_index, c.index_count))
        .collect();
    draws.sort_unstable();
    let want = ranges_of(&dag, &cut(&dag, eye, PPU, THRESHOLD_PX));
    assert_eq!(draws, want, "the retest must draw what the cut selects");
}

/// Benchmark (not run by default): traversal of 1000 instances of a sphere with about 100k triangles.
///
/// Times CPU record + submit + wait per frame, not GPU timestamps. The plan target is 1 ms for the
/// selection pass; the result is reported, not enforced.
#[test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn benchmark_selection_of_a_thousand_sphere_instances() {
    let Some((device, queue)) = gpu() else { return };
    let mesh = create_sphere(1.0, 224);
    let dag = build(&mesh);
    let triangles = mesh.indices.len() / 3;
    let instances: Vec<DagInstance> = (0..1000)
        .map(|i| {
            let x = (i % 40) as f32 * 3.0 - 60.0;
            let z = (i / 40) as f32 * 3.0 - 60.0;
            DagInstance {
                dag: 0,
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [x, 0.0, z, 1.0],
                ],
            }
        })
        .collect();
    let mut traversal = DagTraversal::new(&device, 1 << 20, 1000, 1 << 20, 1 << 21);
    traversal.upload_dags(&queue, std::slice::from_ref(&dag), &[(0, 0)]).unwrap();
    let (_texture, view, sampler) = hiz_with(&device, &queue, 1.0);
    let commands = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vg_dag_bench_commands"),
        size: (1 << 20) * std::mem::size_of::<DrawIndexedIndirectCmd>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let frame = DagFrame {
        instances: &instances,
        eye: [0.0, 30.0, -100.0],
        pixels_per_unit: PPU,
        threshold_px: THRESHOLD_PX,
        cull_frustum: false,
    };
    let frames = 10u32;
    let start = std::time::Instant::now();
    for _ in 0..frames {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        traversal.record(&queue, &device, &mut encoder, &frame, &CullParamsUniform::zeroed(), (&view, &sampler), &commands);
        queue.submit(Some(encoder.finish()));
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
    let per_frame = start.elapsed() / frames;
    println!(
        "[VG-02 bench] {triangles} triangles per instance, 1000 instances: {per_frame:?} per traversal (CPU record + submit + wait)"
    );
}
