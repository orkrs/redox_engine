//! The device is created with at least the wgpu default limits (skipped without a GPU).

mod common;

use redox_render::renderer::required_device_limits;

#[test]
fn the_device_has_at_least_the_default_limits() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let required = required_device_limits(&adapter.limits());
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: required,
        ..Default::default()
    }))
    .expect("device with the required limits");

    let granted = device.limits();
    let defaults = wgpu::Limits::default();
    assert!(
        granted.max_sampled_textures_per_shader_stage
            >= defaults.max_sampled_textures_per_shader_stage
    );
    assert!(
        granted.max_storage_buffers_per_shader_stage
            >= defaults.max_storage_buffers_per_shader_stage
    );
    assert!(
        granted.max_storage_textures_per_shader_stage
            >= defaults.max_storage_textures_per_shader_stage
    );
}
