//! Procedural 64×64 blue-noise R8 texture for stochastic alpha testing.

/// Generate a 64×64 R8 blue-noise tile (4096 bytes).
pub fn generate_blue_noise_64() -> Vec<u8> {
    let size = 64usize;
    let mut data = vec![0u8; size * size];
    for y in 0..size {
        for x in 0..size {
            let h = hash2d(x as u32, y as u32, 0xB10C_u32);
            data[y * size + x] = (h & 0xFF) as u8;
        }
    }
    data
}

fn hash2d(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663) ^ seed;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846ca68b);
    h ^= h >> 16;
    h
}

/// Upload blue-noise texture to the GPU.
pub fn create_blue_noise_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let data = generate_blue_noise_64();
    let size = wgpu::Extent3d {
        width: 64,
        height: 64,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blue_noise_64"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: Some(64),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("blue_noise_sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    (texture, view, sampler)
}
