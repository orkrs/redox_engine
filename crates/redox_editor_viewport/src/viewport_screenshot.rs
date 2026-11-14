//! Read back the viewport colour target to PNG (headless-safe entry point).

use std::path::Path;

/// Encodes `rgba8` (`width`×`height`, row-major) to PNG on disk.
pub fn write_viewport_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    if rgba.len() != (width as usize) * (height as usize) * 4 {
        return Err("rgba length does not match dimensions".into());
    }
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "invalid rgba dimensions".to_string())?;
    img.save(path).map_err(|e| e.to_string())
}

/// Copies an RGBA8 render target to CPU memory (blocking).
pub fn read_rgba8_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let bytes_per_row = width * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = (bytes_per_row + align - 1) / align * align;
    let buffer_size = padded as u64 * height as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport_screenshot_staging"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("viewport_screenshot"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()
        .map_err(|_| "map channel closed".to_string())?
        .map_err(|e| format!("map failed: {e:?}"))?;
    let mapped = slice
        .get_mapped_range()
        .map_err(|err| format!("map range failed: {err:?}"))?;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = row as usize * padded as usize;
        out.extend_from_slice(&mapped[start..start + bytes_per_row as usize]);
    }
    drop(mapped);
    staging.unmap();
    Ok(out)
}

/// Convenience: read + write PNG.
pub fn screenshot_texture_to_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    path: &Path,
) -> Result<(), String> {
    let rgba = read_rgba8_texture(device, queue, texture, width, height)?;
    write_viewport_png(path, width, height, &rgba)
}
