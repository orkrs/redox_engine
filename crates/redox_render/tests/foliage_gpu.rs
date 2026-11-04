//! GPU checks of the foliage culling (FOL-03 step 1): the compute pass must select the same
//! instances and LODs as `cull_reference`, frame after frame. Skipped without a GPU. No renderer
//! is created, only a device.

#![allow(clippy::unwrap_used)]

mod common;

use redox_render::foliage::{
    CullOutcome, FoliageCull, FoliageGpu, FoliageInstance, FoliageLayer, FoliageView, LodRef,
    MAX_LODS, NO_LOD, cull_reference,
};

const FOV_Y: f32 = 1.0;
const NEAR: f32 = 0.1;
const FAR: f32 = 400.0;
const BOUND_RADIUS: f32 = 0.5;

fn lods() -> Vec<LodRef> {
    [20.0, 60.0, 150.0]
        .iter()
        .enumerate()
        .map(|(i, &max_distance)| LodRef {
            mesh_slot: i as u32,
            index_count: 300 >> i,
            max_distance,
        })
        .collect()
}

/// Column-major product `a * b`.
fn mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for (c, column) in out.iter_mut().enumerate() {
        for (r, value) in column.iter_mut().enumerate() {
            *value = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

/// View-projection of a camera at `eye` looking down -Z, with wgpu's [0, 1] depth range.
fn view_for(eye: [f32; 3]) -> FoliageView {
    let f = 1.0 / (FOV_Y * 0.5).tan();
    let proj = [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, FAR / (NEAR - FAR), -1.0],
        [0.0, 0.0, NEAR * FAR / (NEAR - FAR), 0.0],
    ];
    let view = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-eye[0], -eye[1], -eye[2], 1.0],
    ];
    FoliageView {
        view_proj: mul(proj, view),
        camera_pos: eye,
    }
}

/// Deterministic xorshift in `[0, 1)`.
fn unit(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1u64 << 24) as f32
}

fn random_layer(count: usize) -> FoliageLayer {
    let mut layer = FoliageLayer::new(lods(), 0, BOUND_RADIUS).unwrap();
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    for seed in 0..count {
        let pos = [
            unit(&mut s) * 120.0 - 60.0,
            unit(&mut s) * 10.0 - 5.0,
            unit(&mut s) * 220.0 - 200.0,
        ];
        let scale = 0.5 + unit(&mut s);
        layer
            .instances
            .push(FoliageInstance::new(pos, 0.0, scale, seed as u32));
    }
    layer
}

/// What one GPU frame produced: visible `(lod, index)` pairs and the raw indirect records.
struct GpuFrame {
    draws: Vec<(u32, u32)>,
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = common::adapter()?;
    common::device_with_bc(&adapter)
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, src: &wgpu::Buffer, size: u64) -> Vec<u8> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("foliage_test_staging"),
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

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Runs one cull frame on the GPU and reads back the indirect records and the visible indices.
fn run_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cull: &FoliageCull,
    gpu_layer: &FoliageGpu,
    view: &FoliageView,
) -> GpuFrame {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    cull.record(device, queue, &mut encoder, gpu_layer, view);
    queue.submit(Some(encoder.finish()));

    let indirect = words(&read_back(
        device,
        queue,
        gpu_layer.indirect_buffer(),
        MAX_LODS as u64 * 20,
    ));
    let capacity = gpu_layer.capacity() as usize;
    let visible = words(&read_back(
        device,
        queue,
        gpu_layer.visible_buffer(),
        (MAX_LODS * capacity * 4) as u64,
    ));

    let mut draws = Vec::new();
    for lod in 0..gpu_layer.lod_count() {
        let count = indirect[lod * 5 + 1] as usize;
        for slot in 0..count {
            draws.push((lod as u32, visible[lod * capacity + slot]));
        }
    }
    draws.sort_unstable();
    GpuFrame { draws }
}

/// Runs the GPU cull and the CPU reference for `views` in sequence, carrying the LOD state of
/// each frame into the next. Returns both results per frame.
fn run_sequence(
    layer: &FoliageLayer,
    views: &[FoliageView],
) -> Option<Vec<(GpuFrame, CullOutcome)>> {
    let (device, queue) = gpu()?;
    let cull = FoliageCull::new(&device);
    let gpu_layer = FoliageGpu::upload(&device, &queue, layer);
    let mut previous = vec![NO_LOD; layer.instances.len()];
    let mut out = Vec::new();
    for view in views {
        let frame = run_frame(&device, &queue, &cull, &gpu_layer, view);
        let reference = cull_reference(layer, view, &previous);
        previous = reference.lods.clone();
        out.push((frame, reference));
    }
    Some(out)
}

#[test]
fn the_gpu_cull_matches_the_cpu_reference() {
    let layer = random_layer(5_000);
    let views = [
        view_for([0.0, 1.6, 0.0]),
        view_for([0.0, 1.6, -10.0]),
        view_for([0.0, 1.6, -11.0]),
    ];
    let Some(frames) = run_sequence(&layer, &views) else {
        return;
    };
    for (frame_index, (gpu_frame, reference)) in frames.iter().enumerate() {
        let mut want = reference.draws.clone();
        want.sort_unstable();
        assert!(!want.is_empty(), "frame {frame_index} draws nothing");
        assert_eq!(gpu_frame.draws, want, "frame {frame_index}");
    }
}

#[test]
fn instances_behind_the_camera_are_culled() {
    // Camera at the origin looks down -Z: indices 0..100 sit behind it, 100..200 in front.
    let mut layer = FoliageLayer::new(lods(), 0, BOUND_RADIUS).unwrap();
    for i in 0..200 {
        let z = if i < 100 {
            5.0 + i as f32 * 0.5
        } else {
            -5.0 - (i - 100) as f32 * 0.5
        };
        layer
            .instances
            .push(FoliageInstance::new([0.0, 0.0, z], 0.0, 1.0, i));
    }
    let view = view_for([0.0, 0.0, 0.0]);
    let Some(frames) = run_sequence(&layer, &[view]) else {
        return;
    };
    let (gpu_frame, reference) = &frames[0];
    assert!(
        gpu_frame.draws.iter().all(|&(_, index)| index >= 100),
        "a behind-camera instance was drawn: {:?}",
        gpu_frame.draws
    );
    assert!(gpu_frame.draws.iter().any(|&(_, index)| index >= 100));
    assert_eq!(gpu_frame.draws.len(), reference.draws.len());
}

#[test]
fn lod_follows_the_distance_with_hysteresis() {
    // One instance at the origin; the camera moves along +Z, so the distance is the eye's z.
    // Thresholds are 20, 60 and 150 m with a 5 % band: LOD 0 holds up to 21 m, LOD 1 is kept down
    // to 19 m.
    let mut layer = FoliageLayer::new(lods(), 0, BOUND_RADIUS).unwrap();
    layer
        .instances
        .push(FoliageInstance::new([0.0, 0.0, 0.0], 0.0, 1.0, 7));
    let distances = [10.0, 19.5, 20.9, 22.0, 19.5, 18.0];
    let views: Vec<FoliageView> = distances.iter().map(|&d| view_for([0.0, 0.0, d])).collect();
    let Some(frames) = run_sequence(&layer, &views) else {
        return;
    };
    let gpu_lods: Vec<u32> = frames
        .iter()
        .map(|(gpu_frame, _)| gpu_frame.draws.first().map_or(NO_LOD, |&(lod, _)| lod))
        .collect();
    assert_eq!(gpu_lods, vec![0, 0, 0, 1, 1, 0]);
    for (gpu_frame, reference) in &frames {
        let want: Vec<(u32, u32)> = reference.draws.clone();
        assert_eq!(gpu_frame.draws, want);
    }
}
