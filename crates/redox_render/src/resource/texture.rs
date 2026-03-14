//! Texture loading and GPU upload utilities.

/// A GPU texture with its view and sampler, ready to be bound in a shader.
pub struct Texture {
    #[allow(dead_code)]
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub width: u32,
    pub height: u32,
}

impl Texture {
    /// Decodes image bytes (PNG, JPEG, etc.) and uploads the result to the GPU.
    ///
    /// * `format` — can be `Rgba8UnormSrgb` (default for albedo) or `Rgba8Unorm` (for normals/MR).
    /// Decodes image bytes (PNG, JPEG, etc.) and uploads the result to the GPU.
    ///
    /// * `format` — can be `Rgba8UnormSrgb` (default for albedo) or `Rgba8Unorm` (for normals/MR).
    pub fn from_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        label: &str,
        format: wgpu::TextureFormat,
    ) -> Result<Self, image::ImageError> {
        let img = image::load_from_memory(bytes)?;
        Self::from_image(device, queue, &img, label, format)
    }

    /// Creates a texture from a DynamicImage.
    pub fn from_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &image::DynamicImage,
        label: &str,
        format: wgpu::TextureFormat,
    ) -> Result<Self, image::ImageError> {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
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
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("{label}_sampler")),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            texture,
            view,
            sampler,
            width,
            height,
        })
    }

    /// Loads an HDR (Radiance) image and uploads it as Rgba16Float (filterable).
    pub fn from_hdr_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        label: &str,
    ) -> Result<Self, image::ImageError> {
        let decoder = image::codecs::hdr::HdrDecoder::new(bytes)?;
        let metadata = decoder.metadata();
        let (width, height) = (metadata.width, metadata.height);
        let pixels = decoder.read_image_hdr()?;

        // Convert Rgb<f32> to Rgba<f16] packed as u16 for a filterable format.
        let mut data: Vec<u16> = Vec::with_capacity(width as usize * height as usize * 4);
        for pix in pixels {
            data.push(half::f16::from_f32(pix[0]).to_bits());
            data.push(half::f16::from_f32(pix[1]).to_bits());
            data.push(half::f16::from_f32(pix[2]).to_bits());
            data.push(half::f16::from_f32(1.0).to_bits());
        }

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
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
            bytemuck::cast_slice(&data),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(8 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("{label}_sampler")),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            texture,
            view,
            sampler,
            width,
            height,
        })
    }

    /// Creates a 1×1 white texture (fallback for materials without a texture).
    pub fn white_1x1(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::from_bytes(
            device,
            queue,
            &create_pixel_png([255, 255, 255, 255]),
            "white_1x1",
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
        .expect("Failed to create 1x1 white fallback texture")
    }

    /// Creates a 1×1 flat normal map (0.5, 0.5, 1.0 -> [128, 128, 255]).
    pub fn normal_1x1(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::from_bytes(
            device,
            queue,
            &create_pixel_png([128, 128, 255, 255]),
            "normal_1x1",
            wgpu::TextureFormat::Rgba8Unorm, // Linearly interpreted
        )
        .expect("Failed to create 1x1 normal fallback texture")
    }

    /// Creates a 1×1 metallic-roughness map (Metallic=0, Roughness=1.0 -> [255, 255, 0, 255]).
    pub fn mr_1x1(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::from_bytes(
            device,
            queue,
            &create_pixel_png([255, 255, 0, 255]),
            "mr_1x1",
            wgpu::TextureFormat::Rgba8Unorm, // Linearly interpreted
        )
        .expect("Failed to create 1x1 MR fallback texture")
    }
}

/// Produces a minimal 1×1 PNG with the given RGBA color.
fn create_pixel_png(color: [u8; 4]) -> Vec<u8> {
use image::ImageEncoder;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        encoder
            .write_image(&color, 1, 1, image::ColorType::Rgba8)
            .expect("PNG encode failed");
    }
    buf.into_inner()
}
