//! Texture loading and GPU upload utilities.
//!
//! Uploads use **256-byte-aligned staging rows** (wgpu requirement) and
//! generate a full **mipmap chain** via successive blit render passes, so
//! distant surfaces stop shimmering.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;

/// A GPU texture with its view and sampler, ready to be bound in a shader.
pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub width: u32,
    pub height: u32,
    /// Total mip levels generated for this texture.
    pub mip_levels: u32,
}

impl Texture {
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

    /// Creates a texture from a DynamicImage (with mipmaps).
    ///
    /// For `Rgba8Unorm` labels containing `"normal"` (case-insensitive), mip
    /// levels are built with Toksvig filtering and roughness delta in alpha.
    pub fn from_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &image::DynamicImage,
        label: &str,
        format: wgpu::TextureFormat,
    ) -> Result<Self, image::ImageError> {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        let is_normal = format == wgpu::TextureFormat::Rgba8Unorm
            && label.to_ascii_lowercase().contains("normal");

        if is_normal {
            return Ok(Self::upload_normal_toksvig(
                device,
                queue,
                width,
                height,
                rgba.as_raw(),
                label,
            ));
        }

        Ok(Self::upload_2d(
            device,
            queue,
            width,
            height,
            format,
            bytemuck::cast_slice(rgba.as_raw()),
            4,
            label,
        ))
    }

    /// Upload an RGBA8 normal map with Toksvig-aware CPU mip chain.
    pub fn upload_normal_toksvig(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
        label: &str,
    ) -> Self {
        const MAX_DELTA: f32 = 0.5;
        let mips = crate::resource::toksvig::build_normal_mip_chain_toksvig(
            width, height, rgba, MAX_DELTA,
        );
        let mip_levels = mips.len() as u32;
        let format = wgpu::TextureFormat::Rgba8Unorm;

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let mut mip_w = width;
        let mut mip_h = height;
        for (level, data) in mips.iter().enumerate() {
            let unpadded_bpr = (mip_w as usize) * 4;
            let bytes_per_row = align256(unpadded_bpr as u32);
            let extent = wgpu::Extent3d {
                width: mip_w,
                height: mip_h,
                depth_or_array_layers: 1,
            };
            if bytes_per_row as usize == unpadded_bpr {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(mip_h),
                    },
                    extent,
                );
            } else {
                let mut padded = vec![0u8; bytes_per_row as usize * mip_h as usize];
                for row in 0..mip_h as usize {
                    let src = row * unpadded_bpr;
                    let dst = row * bytes_per_row as usize;
                    padded[dst..dst + unpadded_bpr].copy_from_slice(&data[src..src + unpadded_bpr]);
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &padded,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(mip_h),
                    },
                    extent,
                );
            }
            mip_w = (mip_w / 2).max(1);
            mip_h = (mip_h / 2).max(1);
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("{label}_sampler")),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
            width,
            height,
            mip_levels,
        }
    }

    /// Loads an HDR (Radiance) image and uploads it as Rgba16Float (filterable).
    pub fn from_hdr_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        label: &str,
    ) -> Result<Self, image::ImageError> {
        let img =
            image::load_from_memory_with_format(bytes, image::ImageFormat::Hdr)?.into_rgb32f();
        let (width, height) = (img.width(), img.height());
        let pixels = img.pixels();

        // Convert Rgb<f32> to RGBA f16 packed as u16.
        let mut data: Vec<u16> = Vec::with_capacity(width as usize * height as usize * 4);
        for pix in pixels {
            data.push(half::f16::from_f32(pix[0]).to_bits());
            data.push(half::f16::from_f32(pix[1]).to_bits());
            data.push(half::f16::from_f32(pix[2]).to_bits());
            data.push(half::f16::from_f32(1.0).to_bits());
        }

        Ok(Self::upload_2d(
            device,
            queue,
            width,
            height,
            wgpu::TextureFormat::Rgba16Float,
            bytemuck::cast_slice(&data),
            8, // four f16 channels; passing 2 made every HDR upload fail validation
            label,
        ))
    }

    /// Core upload path: padded staging copy + mip chain + default sampler.
    ///
    /// `bytes_per_pixel` is 4 for Rgba8* and 8 for Rgba16Float.
    #[allow(clippy::too_many_arguments)]
    pub fn upload_2d(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        data: &[u8],
        bytes_per_pixel: usize,
        label: &str,
    ) -> Self {
        let mip_levels = width.max(height).max(1).ilog2() + 1;

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            // RENDER_ATTACHMENT is required by the blit-based
            // mip generation (no copy ops needed).
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        // ── Padded staging upload (bytes_per_row must be 256-aligned) ──
        let unpadded_bpr = width as usize * bytes_per_pixel;
        let bytes_per_row = align256(unpadded_bpr as u32);
        if bytes_per_row as usize == unpadded_bpr {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as _),
                    rows_per_image: Some(height),
                },
                size,
            );
        } else {
            // Copy row-by-row into a padded staging buffer.
            let h = height as usize;
            let row_len = unpadded_bpr;
            let mut padded = vec![0u8; bytes_per_row as usize * h];
            for row in 0..h {
                padded[row * bytes_per_row as usize..row * bytes_per_row as usize + row_len]
                    .copy_from_slice(&data[row * row_len..(row + 1) * row_len]);
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &padded,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as _),
                    rows_per_image: Some(height),
                },
                size,
            );
        }

        // ── Mip chain via successive blit passes ──
        if mip_levels > 1 {
            generate_mipmaps(device, queue, &texture, mip_levels, format);
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("{label}_sampler")),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
            width,
            height,
            mip_levels,
        }
    }

    /// Creates a solid-color 1×1 texture without any PNG round-tripping.
    pub fn solid_1x1(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: [u8; 4],
        label: &str,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self::upload_2d(device, queue, 1, 1, format, &color, 4, label)
    }

    /// Creates a 1×1 white texture (fallback for materials without a texture).
    pub fn white_1x1(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::solid_1x1(
            device,
            queue,
            [255, 255, 255, 255],
            "white_1x1",
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )
    }

    /// Creates a 1×1 flat normal map (0.5, 0.5, 1.0 -> [128, 128, 255]).
    pub fn normal_1x1(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::solid_1x1(
            device,
            queue,
            [128, 128, 255, 255],
            "normal_1x1",
            wgpu::TextureFormat::Rgba8Unorm,
        )
    }

    /// Creates a 1×1 metallic-roughness map (Metallic=0, Roughness=1.0 -> [255, 255, 0, 255]).
    pub fn mr_1x1(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::solid_1x1(
            device,
            queue,
            [255, 255, 0, 255],
            "mr_1x1",
            wgpu::TextureFormat::Rgba8Unorm,
        )
    }
}

/// Rounds a byte count up to the next multiple of 256 (wgpu alignment).
fn align256(v: u32) -> u32 {
    v.div_ceil(256) * 256
}

/// The WGSL source used for downsampling each mip level.
///
/// UV is derived from the fullscreen-triangle NDC positions in the vertex
/// shader, so it maps exactly onto whatever framebuffer the pass targets —
/// no target-dimension uniform required.
const BLIT_SHADER_SRC: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
struct Out { @location(0) color: vec4<f32> };

@vertex fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-3.0, 1.0), vec2(3.0, 1.0));
    var o: VOut;
    o.pos = vec4(p[vi], 0.0, 1.0);
    // NDC -> UV (y flipped for framebuffer orientation); oversized triangle
    // covers every pixel, sampler clamp handles the overhang.
    o.uv = p[vi] * vec2(0.5, -0.5) + vec2(0.5, 0.5);
    return o;
}

@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(in: VOut) -> Out {
    var out: Out;
    out.color = textureSample(t, s, in.uv);
    return out;
}
"#;

fn blit_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> Arc<wgpu::RenderPipeline> {
    static CACHE: LazyLock<Mutex<HashMap<wgpu::TextureFormat, Arc<wgpu::RenderPipeline>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut guard = CACHE.lock();
    let key = format;
    if let Some(p) = guard.get(&key) {
        return p.clone();
    }

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit_mipmap_shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SHADER_SRC.into()),
    });
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("blit_mipmap_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("blit_mipmap_layout"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = Arc::new(
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("blit_mipmap_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        }),
    );
    guard.insert(key, pipeline.clone());
    pipeline
}

/// Generates the full mip chain of `texture` by successively downsampling the
/// previous level through a linear-filtered fullscreen blit.
fn generate_mipmaps(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip_levels: u32,
    format: wgpu::TextureFormat,
) {
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("mipmap_blit_sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let pipeline = blit_pipeline(device, format);
    let bgl = pipeline.get_bind_group_layout(0);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("mipmap_blit_encoder"),
    });

    for level in 1..mip_levels {
        let src_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("mipmap_src"),
            base_mip_level: level - 1,
            mip_level_count: Some(1),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });
        let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("mipmap_dst"),
            base_mip_level: level,
            mip_level_count: Some(1),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mipmap_blit_bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mipmap_blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: &dst_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
    }

    queue.submit(std::iter::once(encoder.finish()));
}
