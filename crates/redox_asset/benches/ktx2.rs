//! KTX2 loading: `cargo bench -p redox_asset --bench ktx2`.

#![allow(clippy::unwrap_used)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use redox_asset::{AssetLoader, Ktx2Loader, Ktx2Texture, TextureFormat};

/// A BC7-sized texture whose blocks repeat in a pattern (compressible like real texture data).
fn texture(size: u32) -> Ktx2Texture {
    let mut levels = Vec::new();
    let mut s = size;
    loop {
        let bytes = TextureFormat::Bc7Unorm.level_size(s, s);
        let mut data = Vec::with_capacity(bytes);
        let mut state = 0x1234_5678_u32;
        while data.len() < bytes {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            // 16-byte blocks drawn from a small palette.
            let block = (state >> 24) % 64;
            data.extend((0..16u32).map(|i| (block * 5 + i * 3) as u8));
        }
        data.truncate(bytes);
        levels.push(data);
        if s == 4 {
            break;
        }
        s /= 2;
    }
    Ktx2Texture::new_2d(size, size, TextureFormat::Bc7Unorm, levels)
}

fn bench_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("ktx2_load_bc7");
    group.sample_size(10);
    for size in [1024u32, 4096] {
        let tex = texture(size);
        let plain = tex.to_ktx2_bytes(false);
        let packed = tex.to_ktx2_bytes(true);
        group.bench_function(format!("{size}px_stored"), |b| {
            b.iter(|| black_box(Ktx2Loader::default().load(black_box(&plain)).unwrap()))
        });
        group.bench_function(
            format!("{size}px_zstd_{}pct", packed.len() * 100 / plain.len()),
            |b| b.iter(|| black_box(Ktx2Loader::default().load(black_box(&packed)).unwrap())),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_load);
criterion_main!(benches);
