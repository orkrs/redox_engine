//! Helpers for creating `wgpu` buffers.

use wgpu::util::DeviceExt;

/// Creates a vertex buffer initialised with `data`.
pub fn create_vertex_buffer(device: &wgpu::Device, label: &str, data: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: data,
        // COPY_DST lets skinned / deforming meshes stream new vertices into the same buffer;
        // STORAGE + COPY_SRC let the GPU skinning pass write it and tools read it back.
        usage: wgpu::BufferUsages::VERTEX
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::STORAGE,
    })
}

/// Creates an index buffer initialised with `data`.
pub fn create_index_buffer(device: &wgpu::Device, label: &str, data: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: data,
        usage: wgpu::BufferUsages::INDEX,
    })
}

/// Creates a uniform buffer initialised with `data`.
///
/// The buffer is created with both `UNIFORM` and `COPY_DST` usages so that
/// it can be updated each frame via `Queue::write_buffer`.
pub fn create_uniform_buffer(device: &wgpu::Device, label: &str, data: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: data,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}
