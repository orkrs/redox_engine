//! Shared GPU test setup (tests are skipped when no adapter exists).
//!
//! [`lock_gpu`] serializes every GPU integration test in the process and across parallel
//! `cargo test` binaries via a temp-dir flock file. Without it, multiple renderers on the same
//! adapter panic inside wgpu when tests run in parallel.

#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::Mutex;

/// Holds the cross-process GPU test lock until dropped.
pub struct GpuTestLock {
    /// Present only for the outermost guard in this process (`flock` is not re-entrant).
    _file: Option<File>,
}

static GPU_LOCK_DEPTH: Mutex<u32> = Mutex::new(0);

fn lock_path() -> PathBuf {
    std::env::temp_dir().join("redox_gpu_test.lock")
}

/// Serializes GPU tests (threads and separate test binaries).
pub fn lock_gpu() -> GpuTestLock {
    let mut depth = GPU_LOCK_DEPTH.lock().unwrap_or_else(|e| e.into_inner());
    if *depth == 0 {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(lock_path())
            .expect("open gpu test lock");
        flock_exclusive(&file);
        *depth = 1;
        GpuTestLock { _file: Some(file) }
    } else {
        *depth += 1;
        GpuTestLock { _file: None }
    }
}

impl Drop for GpuTestLock {
    fn drop(&mut self) {
        let mut depth = GPU_LOCK_DEPTH.lock().unwrap_or_else(|e| e.into_inner());
        assert!(*depth > 0, "GpuTestLock drop with zero depth");
        *depth -= 1;
        if *depth == 0 {
            if let Some(file) = self._file.take() {
                flock_unlock(&file);
            }
        }
    }
}

#[cfg(unix)]
fn flock_exclusive(file: &File) {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };
    assert_eq!(
        ret,
        0,
        "flock LOCK_EX failed: {}",
        std::io::Error::last_os_error()
    );
}

#[cfg(unix)]
fn flock_unlock(file: &File) {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    let _ = unsafe { libc::flock(fd, libc::LOCK_UN) };
}

#[cfg(not(unix))]
fn flock_exclusive(_file: &File) {}

#[cfg(not(unix))]
fn flock_unlock(_file: &File) {}

pub fn adapter() -> Option<wgpu::Adapter> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
}

pub fn device_with_bc(adapter: &wgpu::Adapter) -> Option<(wgpu::Device, wgpu::Queue)> {
    let wanted = wgpu::Features::PRIMITIVE_INDEX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::TEXTURE_COMPRESSION_BC
        | wgpu::Features::PIPELINE_CACHE
        | wgpu::Features::INDIRECT_FIRST_INSTANCE
        | redox_render::profiler::PROFILER_FEATURES;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        required_limits: if adapter.get_info().backend == wgpu::Backend::Vulkan {
            redox_render::renderer::required_device_limits(&adapter.limits())
        } else {
            wgpu::Limits::default()
        },
        ..Default::default()
    }))
    .ok()
}

pub fn renderer(adapter: &wgpu::Adapter, size: u32) -> Option<redox_render::Renderer> {
    renderer_sized(adapter, size, size)
}

/// A renderer whose surface is `width` x `height` (for tests that need a non-square size).
pub fn renderer_sized(
    adapter: &wgpu::Adapter,
    width: u32,
    height: u32,
) -> Option<redox_render::Renderer> {
    let (device, queue) = device_with_bc(adapter)?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    Some(redox_render::Renderer::from_shared_device(
        device, queue, adapter, None, config,
    ))
}

pub fn target(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
