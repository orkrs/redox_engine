//! Readback of one finished frame: numbers for every target the chain writes.
//!
//! Used by `tests/chain_probe.rs`. Each sample is the texture after `render_frame_view`,
//! so the order below is the order the passes ran.

use super::Renderer;

/// One target, reduced to a few numbers plus an 8×8 luminance map.
#[derive(Clone, Debug)]
pub struct ChainSample {
    /// Target name, including which slot is the TAA output.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Centre pixel. Depth stores the raw depth in `x` and leaves the rest at 0.
    pub center: [f32; 4],
    /// Pixel (1, 1), the background when the cube sits in the middle.
    pub corner: [f32; 4],
    /// Mean of `0.2126 r + 0.7152 g + 0.0722 b`. For depth this is the mean raw depth.
    pub mean_luma: f32,
    /// Max of the same luma. For depth this is the minimum raw depth (the nearest pixel).
    pub max_luma: f32,
    /// Fraction of pixels brighter than 0.08. For depth, fraction closer than 0.999.
    pub lit_fraction: f32,
    /// 8×8 block averages, one character each: `.` then `1`–`9`.
    pub map: String,
}

/// Every target sampled after a frame, plus the binding check.
pub struct ChainReport {
    /// Multi-line text, one numbered step per target.
    pub text: String,
    /// Resolved HDR the forward pass drew.
    pub hdr: ChainSample,
    /// TAA history slot written this frame (what the post chain reads).
    pub taa_out: ChainSample,
    /// Post-effect target 0. With SSR + motion + fog this is the chain output.
    pub post0: ChainSample,
    /// Post-effect target 1, the other ping-pong slot.
    pub post1: ChainSample,
    /// Tonemapped image in the caller's target. `None` when no present texture was passed.
    pub present: Option<ChainSample>,
    /// `current` or `stale` for the HDR texture the TAA group was rebuilt against.
    pub hdr_binding: &'static str,
    /// `current` or `stale` for the depth texture.
    pub depth_binding: &'static str,
}

impl Renderer {
    /// Copies HDR, both TAA histories, depth, normals, AO, both post targets and `present`.
    ///
    /// Call it after `render_frame_view`. `present` is the texture that frame was drawn into.
    pub fn frame_chain_report(&self, present: Option<&wgpu::Texture>) -> ChainReport {
        let (hdr_binding, depth_binding) = self.frame_binding_probe();
        let read = self.taa_history_read_idx;
        let mut jobs = vec![
            ("01 depth (prepass)", &self.depth_texture),
            ("02 normals (prepass)", &self.normal_texture),
            ("03 ao (gtao)", &self.ssao_blurred_texture),
            ("04 hdr (forward + msaa resolve)", &self.hdr_texture),
            ("05 taa history 0", &self.taa_history_tex[0]),
            ("06 taa history 1", &self.taa_history_tex[1]),
            ("07 post target 0", &self.post_fx.target_textures()[0]),
            ("08 post target 1", &self.post_fx.target_textures()[1]),
        ];
        if let Some(present) = present {
            jobs.push(("09 present (tonemap)", present));
        }
        let copied = copy_textures(&self.device, &self.queue, &jobs);
        let mut samples = Vec::new();
        for ((name, texture), bytes) in jobs.iter().zip(copied) {
            let mut sample = sample_texture(name, texture, &bytes);
            if name.starts_with("05") && read == 0 || name.starts_with("06") && read == 1 {
                sample.name = format!("{name}  ← TAA output, post reads this");
            }
            samples.push(sample);
        }
        let normals_aliased = self.normal_texture == self.hdr_texture;
        let mut text = format!(
            "path taa={} sharp={} msaa={} size={}×{} read_slot={read} hdr_binding={hdr_binding} depth_binding={depth_binding} normals_aliased_with_hdr={normals_aliased}\n",
            self.taa_enabled,
            self.aa_sharp,
            self.msaa_sample_count,
            self.internal_width,
            self.internal_height,
        );
        for sample in &samples {
            text.push_str(&format_sample(sample));
        }
        let find = |prefix: &str| {
            samples
                .iter()
                .find(|s| s.name.starts_with(prefix))
                .cloned()
                .expect(prefix)
        };
        ChainReport {
            text,
            hdr: find("04"),
            taa_out: find(if read == 0 { "05" } else { "06" }),
            post0: find("07"),
            post1: find("08"),
            present: samples.iter().find(|s| s.name.starts_with("09")).cloned(),
            hdr_binding,
            depth_binding,
        }
    }
}

fn format_sample(sample: &ChainSample) -> String {
    format!(
        "{name}\n  size {w}×{h}  center [{c0:.4}, {c1:.4}, {c2:.4}, {c3:.4}]  corner [{k0:.4}, {k1:.4}, {k2:.4}, {k3:.4}]\n  mean {mean:.4}  peak {peak:.4}  lit {lit:.1}%\n  map\n{map}\n",
        name = sample.name,
        w = sample.width,
        h = sample.height,
        c0 = sample.center[0],
        c1 = sample.center[1],
        c2 = sample.center[2],
        c3 = sample.center[3],
        k0 = sample.corner[0],
        k1 = sample.corner[1],
        k2 = sample.corner[2],
        k3 = sample.corner[3],
        mean = sample.mean_luma,
        peak = sample.max_luma,
        lit = sample.lit_fraction * 100.0,
        map = sample.map,
    )
}

fn sample_texture(name: &str, texture: &wgpu::Texture, bytes: &[u8]) -> ChainSample {
    let width = texture.width();
    let height = texture.height();
    let format = texture.format();
    let depth = format.has_depth_aspect();
    let (center, corner, mean_luma, max_luma, lit_fraction, map) = if depth {
        depth_stats(bytes, width, height)
    } else if texel_bytes(format) == 8 {
        rgba16_stats(bytes, width, height)
    } else {
        rgba8_stats(bytes, width, height)
    };
    ChainSample {
        name: name.to_string(),
        width,
        height,
        center,
        corner,
        mean_luma,
        max_luma,
        lit_fraction,
        map,
    }
}

fn depth_stats(bytes: &[u8], width: u32, height: u32) -> Stats {
    let pixel = |x: u32, y: u32| -> f32 {
        let i = ((y * width + x) * 4) as usize;
        f32::from_le_bytes(bytes[i..i + 4].try_into().unwrap_or([0; 4]))
    };
    let mut sum = 0.0f32;
    let mut nearest = 1.0f32;
    let mut close = 0u32;
    let mut blocks = [[0.0f32; 8]; 8];
    let mut counts = [[0u32; 8]; 8];
    let n = (width * height).max(1) as f32;
    for y in 0..height {
        for x in 0..width {
            let d = pixel(x, y);
            sum += d;
            nearest = nearest.min(d);
            if d < 0.999 {
                close += 1;
            }
            let bx = (x * 8 / width).min(7) as usize;
            let by = (y * 8 / height).min(7) as usize;
            blocks[by][bx] += 1.0 - d;
            counts[by][bx] += 1;
        }
    }
    (
        [pixel(width / 2, height / 2), 0.0, 0.0, 0.0],
        [pixel(1, 1), 0.0, 0.0, 0.0],
        sum / n,
        nearest,
        close as f32 / n,
        map_from(&blocks, &counts, (1.0 - nearest).max(0.002)),
    )
}

fn rgba16_stats(bytes: &[u8], width: u32, height: u32) -> Stats {
    let pixel = |x: u32, y: u32| -> [f32; 4] {
        let i = ((y * width + x) * 8) as usize;
        let mut out = [0.0; 4];
        for c in 0..4 {
            let h =
                u16::from_le_bytes(bytes[i + c * 2..i + c * 2 + 2].try_into().unwrap_or([0; 2]));
            out[c] = f16_to_f32(h);
        }
        out
    };
    color_stats(width, height, pixel)
}

fn rgba8_stats(bytes: &[u8], width: u32, height: u32) -> Stats {
    let pixel = |x: u32, y: u32| -> [f32; 4] {
        let i = ((y * width + x) * 4) as usize;
        [
            bytes[i] as f32 / 255.0,
            bytes.get(i + 1).copied().unwrap_or(0) as f32 / 255.0,
            bytes.get(i + 2).copied().unwrap_or(0) as f32 / 255.0,
            bytes.get(i + 3).copied().unwrap_or(0) as f32 / 255.0,
        ]
    };
    color_stats(width, height, pixel)
}

type Stats = ([f32; 4], [f32; 4], f32, f32, f32, String);

fn color_stats(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [f32; 4]) -> Stats {
    let mut sum = 0.0f32;
    let mut peak = 0.0f32;
    let mut lit = 0u32;
    let mut blocks = [[0.0f32; 8]; 8];
    let mut counts = [[0u32; 8]; 8];
    let n = (width * height).max(1) as f32;
    for y in 0..height {
        for x in 0..width {
            let p = pixel(x, y);
            let luma = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            sum += luma;
            peak = peak.max(luma);
            if luma > 0.08 {
                lit += 1;
            }
            let bx = (x * 8 / width).min(7) as usize;
            let by = (y * 8 / height).min(7) as usize;
            blocks[by][bx] += luma;
            counts[by][bx] += 1;
        }
    }
    let scale = (peak * 0.85).max(0.15);
    (
        pixel(width / 2, height / 2),
        pixel(1.min(width - 1), 1.min(height - 1)),
        sum / n,
        peak,
        lit as f32 / n,
        map_from(&blocks, &counts, scale),
    )
}

fn map_from(blocks: &[[f32; 8]; 8], counts: &[[u32; 8]; 8], scale: f32) -> String {
    let mut out = String::new();
    for y in 0..8 {
        out.push_str("  ");
        for x in 0..8 {
            let avg = blocks[y][x] / counts[y][x].max(1) as f32;
            let digit = ((avg / scale) * 9.0).clamp(0.0, 9.0) as u32;
            out.push(if digit == 0 {
                '.'
            } else {
                char::from(b'0' + digit as u8)
            });
        }
        out.push('\n');
    }
    out
}

fn copy_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    jobs: &[(&str, &wgpu::Texture)],
) -> Vec<Vec<u8>> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("chain_probe_copy"),
    });
    let mut pending = Vec::new();
    for (_, texture) in jobs {
        let format = texture.format();
        let bpp = texel_bytes(format);
        let width = texture.width();
        let height = texture.height();
        let row = width * bpp;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chain_probe_buffer"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: if format.has_depth_aspect() {
                    wgpu::TextureAspect::DepthOnly
                } else {
                    wgpu::TextureAspect::All
                },
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
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
        pending.push((buffer, row, padded, height));
    }
    queue.submit(Some(encoder.finish()));
    for (buffer, ..) in &pending {
        buffer.slice(..).map_async(wgpu::MapMode::Read, |result| {
            result.expect("map chain probe buffer");
        });
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    pending
        .into_iter()
        .map(|(buffer, row, padded, height)| {
            let mapped = buffer
                .slice(..)
                .get_mapped_range()
                .expect("chain probe buffer mapped");
            let mut tight = Vec::with_capacity((row * height) as usize);
            for y in 0..height {
                let start = (y * padded) as usize;
                tight.extend_from_slice(&mapped[start..start + row as usize]);
            }
            drop(mapped);
            buffer.unmap();
            tight
        })
        .collect()
}

fn texel_bytes(format: wgpu::TextureFormat) -> u32 {
    match format {
        wgpu::TextureFormat::Rgba16Float => 8,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => 4,
        wgpu::TextureFormat::Depth32Float | wgpu::TextureFormat::R32Float => 4,
        other => panic!("chain probe cannot read {other:?}"),
    }
}

fn f16_to_f32(h: u16) -> f32 {
    let sign = (h >> 15) & 1;
    let exp = (h >> 10) & 0x1f;
    let frac = h & 0x3ff;
    let mag = match exp {
        0 => f32::from(frac) * 2.0f32.powi(-24),
        31 => {
            if frac == 0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => (1.0 + f32::from(frac) / 1024.0) * 2.0f32.powi(i32::from(e) - 15),
    };
    if sign == 1 { -mag } else { mag }
}
