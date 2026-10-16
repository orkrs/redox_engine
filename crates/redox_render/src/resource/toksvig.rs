//! Toksvig normal-map filtering for mip generation.
//!
//! When normals are averaged into a lower mip, `‖N̄‖ < 1`. Toksvig maps that
//! length loss to an additive roughness term so distant specular stays quiet:
//! `σ² = (1 − ‖N̄‖) / ‖N̄‖`.

/// Variance from the length of an averaged unit normal (Toksvig / vMF).
#[inline]
pub fn toksvig_variance(avg_normal_len: f32) -> f32 {
    let len = avg_normal_len.max(1e-4);
    ((1.0 - len) / len).max(0.0)
}

/// Convert Toksvig variance into a perceptual-roughness delta, clamped.
#[inline]
pub fn toksvig_roughness_delta(avg_normal_len: f32, max_delta: f32) -> f32 {
    let v = toksvig_variance(avg_normal_len);
    // alpha² += variance → perceptual roughly sqrt; keep a soft clamp.
    (v.sqrt() * 0.5).min(max_delta)
}

/// Decode an RGBA8 normal (tangent-space, [0,1] → [-1,1]), return unit vector.
#[inline]
pub fn decode_normal_rgba8(px: [u8; 4]) -> [f32; 3] {
    let x = (px[0] as f32 / 255.0) * 2.0 - 1.0;
    let y = (px[1] as f32 / 255.0) * 2.0 - 1.0;
    let z = (px[2] as f32 / 255.0) * 2.0 - 1.0;
    let len = (x * x + y * y + z * z).sqrt().max(1e-6);
    [x / len, y / len, z / len]
}

/// Encode a unit normal back to RGBA8; alpha stores Toksvig roughness delta
/// in [0,1] (0 = sharp, 1 = max_delta mapped).
#[inline]
pub fn encode_normal_rgba8(n: [f32; 3], roughness_delta: f32, max_delta: f32) -> [u8; 4] {
    let scale = max_delta.max(1e-6);
    [
        ((n[0] * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((n[1] * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((n[2] * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((roughness_delta / scale).clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

/// Build a full Toksvig-aware mip chain from an RGBA8 normal map (mip 0).
///
/// Returns `mips[level]` as tightly packed RGBA8 bytes. Mip 0 alpha is left
/// unchanged (or 255). Higher mips store Toksvig roughness delta in alpha.
pub fn build_normal_mip_chain_toksvig(
    width: u32,
    height: u32,
    rgba: &[u8],
    max_roughness_delta: f32,
) -> Vec<Vec<u8>> {
    assert_eq!(rgba.len(), (width as usize) * (height as usize) * 4);
    let mip_levels = width.max(height).max(1).ilog2() + 1;
    let mut mips: Vec<Vec<u8>> = Vec::with_capacity(mip_levels as usize);
    mips.push(rgba.to_vec());

    let mut prev_w = width as usize;
    let mut prev_h = height as usize;

    for _level in 1..mip_levels {
        let w = (prev_w / 2).max(1);
        let h = (prev_h / 2).max(1);
        let prev = &mips[mips.len() - 1];
        let mut out = vec![0u8; w * h * 4];

        for y in 0..h {
            for x in 0..w {
                let x0 = (x * 2).min(prev_w - 1);
                let y0 = (y * 2).min(prev_h - 1);
                let x1 = (x0 + 1).min(prev_w - 1);
                let y1 = (y0 + 1).min(prev_h - 1);

                let samples = [
                    read_px(prev, prev_w, x0, y0),
                    read_px(prev, prev_w, x1, y0),
                    read_px(prev, prev_w, x0, y1),
                    read_px(prev, prev_w, x1, y1),
                ];

                let mut sx = 0.0f32;
                let mut sy = 0.0f32;
                let mut sz = 0.0f32;
                for s in samples {
                    let n = decode_normal_rgba8(s);
                    sx += n[0];
                    sy += n[1];
                    sz += n[2];
                }
                sx *= 0.25;
                sy *= 0.25;
                sz *= 0.25;
                let avg_len = (sx * sx + sy * sy + sz * sz).sqrt();
                let inv = if avg_len > 1e-6 { 1.0 / avg_len } else { 1.0 };
                let n = [sx * inv, sy * inv, sz * inv];
                let delta = toksvig_roughness_delta(avg_len, max_roughness_delta);
                let enc = encode_normal_rgba8(n, delta, max_roughness_delta);
                let i = (y * w + x) * 4;
                out[i..i + 4].copy_from_slice(&enc);
            }
        }

        mips.push(out);
        prev_w = w;
        prev_h = h;
    }

    mips
}

#[inline]
fn read_px(buf: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * width + x) * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_normal_has_zero_variance() {
        assert!(toksvig_variance(1.0) < 1e-5);
    }

    #[test]
    fn short_normal_raises_roughness() {
        let d = toksvig_roughness_delta(0.5, 0.5);
        assert!(d > 0.1);
    }

    #[test]
    fn mip_chain_shrinks() {
        let w = 4u32;
        let h = 4u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        // Flat +Z normals
        for px in rgba.chunks_exact_mut(4) {
            px[0] = 128;
            px[1] = 128;
            px[2] = 255;
            px[3] = 255;
        }
        let mips = build_normal_mip_chain_toksvig(w, h, &rgba, 0.5);
        assert_eq!(mips.len(), 3); // 4→2→1
        assert_eq!(mips[1].len(), 2 * 2 * 4);
        assert_eq!(mips[2].len(), 4);
    }
}
