//! Basis Universal: `.basis` files and KTX2 containers with UASTC and BasisLZ data.
//!
//! The sample files come from the `basis-universal` crate (the same 64x64 Rust logo, encoded as
//! ETC1S and as UASTC), so quality is measured against the original PNG.

#![allow(clippy::unwrap_used)]

use redox_asset::{
    AssetLoader, BasisTarget, Ktx2Loader, Ktx2Texture, LoadOptions, TextureFormat,
    basis_file_to_ktx2,
};

const ETC: &[u8] = include_bytes!("data/rust-logo-etc.basis");
const UASTC: &[u8] = include_bytes!("data/rust-logo-uastc.basis");
const PNG: &[u8] = include_bytes!("data/rust-logo.png");

fn loader(target: BasisTarget, srgb: bool) -> Ktx2Loader {
    Ktx2Loader::with_options(LoadOptions {
        basis_target: target,
        srgb,
    })
}

/// PSNR in dB over all four channels between the transcoded base level and the PNG.
fn psnr(texture: &Ktx2Texture) -> f64 {
    assert!(matches!(
        texture.format,
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8Srgb
    ));
    let reference = image::load_from_memory(PNG).unwrap().to_rgba8();
    assert_eq!(
        (reference.width(), reference.height()),
        (texture.width, texture.height)
    );
    let decoded = &texture.levels[0];
    let sum: f64 = decoded
        .iter()
        .zip(reference.as_raw())
        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum();
    let mse = sum / decoded.len() as f64;
    if mse == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0f64 * 255.0 / mse).log10()
}

#[test]
fn etc1s_and_uastc_files_transcode_to_rgba8_close_to_the_original() {
    let etc = loader(BasisTarget::Rgba8, true).load(ETC).unwrap();
    let uastc = loader(BasisTarget::Rgba8, true).load(UASTC).unwrap();
    assert_eq!((etc.width, etc.height, etc.levels.len()), (64, 64, 7));
    assert_eq!((uastc.width, uastc.height, uastc.levels.len()), (64, 64, 7));
    assert_eq!(etc.format, TextureFormat::Rgba8Srgb);
    let (etc_db, uastc_db) = (psnr(&etc), psnr(&uastc));
    eprintln!("PSNR: ETC1S {etc_db:.1} dB, UASTC {uastc_db:.1} dB");
    assert!(etc_db > 20.0, "ETC1S too lossy: {etc_db}");
    assert!(uastc_db > 28.0, "UASTC too lossy: {uastc_db}");
    assert!(uastc_db > etc_db, "UASTC is the higher quality mode");
    for tex in [&etc, &uastc] {
        for mip in 0..tex.levels.len() {
            assert_eq!(tex.levels[mip].len(), tex.subimage_size(mip));
        }
    }
}

#[test]
fn compressed_targets_have_the_right_sizes_and_formats() {
    for (target, linear, srgb) in [
        (
            BasisTarget::Bc7,
            TextureFormat::Bc7Unorm,
            TextureFormat::Bc7Srgb,
        ),
        (
            BasisTarget::Bc3,
            TextureFormat::Bc3Unorm,
            TextureFormat::Bc3Srgb,
        ),
        // The sample has alpha, so BC1 is replaced by BC3.
        (
            BasisTarget::Bc1,
            TextureFormat::Bc3Unorm,
            TextureFormat::Bc3Srgb,
        ),
    ] {
        for file in [ETC, UASTC] {
            let tex = loader(target, false).load(file).unwrap();
            assert_eq!(tex.format, linear, "{target:?}");
            assert_eq!(loader(target, true).load(file).unwrap().format, srgb);
            assert_eq!(tex.levels.len(), 7);
            for (mip, level) in tex.levels.iter().enumerate() {
                assert_eq!(
                    level.len(),
                    tex.format
                        .level_size((64 >> mip).max(1), (64 >> mip).max(1))
                );
            }
        }
    }
}

#[test]
fn ktx2_uastc_matches_the_basis_file_with_and_without_zstd() {
    let direct = loader(BasisTarget::Rgba8, false).load(UASTC).unwrap();
    for zstd in [false, true] {
        let ktx2 = basis_file_to_ktx2(UASTC, zstd).unwrap();
        let tex = loader(BasisTarget::Rgba8, true).load(&ktx2).unwrap();
        // The file's own descriptor says linear (the sample has no sRGB flag).
        assert_eq!(tex.format, TextureFormat::Rgba8Unorm);
        assert_eq!(tex.levels, direct.levels, "zstd={zstd}");
    }
    let plain = basis_file_to_ktx2(UASTC, false).unwrap();
    let packed = basis_file_to_ktx2(UASTC, true).unwrap();
    assert!(
        packed.len() < plain.len(),
        "{} vs {}",
        packed.len(),
        plain.len()
    );
}

#[test]
fn ktx2_basislz_matches_the_basis_file() {
    let direct = loader(BasisTarget::Rgba8, false).load(ETC).unwrap();
    let ktx2 = basis_file_to_ktx2(ETC, false).unwrap();
    let tex = loader(BasisTarget::Rgba8, false).load(&ktx2).unwrap();
    assert_eq!(
        tex.levels, direct.levels,
        "ETC1S survived the KTX2 wrapper unchanged"
    );
    // And the container reads as KTX2 with BasisLZ and the ETC1S colour model.
    let reader = ktx2::Reader::new(ktx2.as_slice()).unwrap();
    assert_eq!(
        reader.header().supercompression_scheme,
        Some(ktx2::SupercompressionScheme::BasisLZ)
    );
    assert_eq!(
        reader.basic_dfd().unwrap().color_model,
        Some(ktx2::ColorModel::ETC1S)
    );
}

#[test]
fn basis_targets_work_through_ktx2_too() {
    for file in [ETC, UASTC] {
        let ktx2 = basis_file_to_ktx2(file, true).unwrap();
        let bc7 = loader(BasisTarget::Bc7, true).load(&ktx2).unwrap();
        let direct = loader(BasisTarget::Bc7, false).load(file).unwrap();
        assert_eq!(bc7.levels, direct.levels);
        assert_eq!(
            bc7.format,
            TextureFormat::Bc7Unorm,
            "descriptor says linear"
        );
    }
}

/// Flips every third byte and cuts the file at every fifth length; nothing may crash.
fn survives_damage(file: &[u8]) {
    let loader = loader(BasisTarget::Rgba8, true);
    for cut in (0..file.len()).step_by(5) {
        let _ = loader.load(&file[..cut]);
    }
    for i in (0..file.len()).step_by(3) {
        let mut broken = file.to_vec();
        broken[i] ^= 0x5A;
        let _ = loader.load(&broken);
    }
}

#[test]
fn damaged_basis_files_are_rejected_by_their_checksums() {
    survives_damage(ETC);
    survives_damage(UASTC);
    // Every single flipped byte of a .basis file is caught (CRC16 detects all 1-byte errors).
    for i in 0..ETC.len() {
        let mut broken = ETC.to_vec();
        broken[i] ^= 0x01;
        assert!(
            loader(BasisTarget::Rgba8, true).load(&broken).is_err(),
            "byte {i}"
        );
    }
}

#[test]
fn damaged_ktx2_uastc_files_do_not_crash() {
    survives_damage(&basis_file_to_ktx2(UASTC, true).unwrap());
    survives_damage(&basis_file_to_ktx2(UASTC, false).unwrap());
}

#[test]
fn damaged_ktx2_basislz_files_do_not_crash() {
    survives_damage(&basis_file_to_ktx2(ETC, false).unwrap());
}

#[test]
fn files_can_be_loaded_through_the_asset_manager_by_extension() {
    assert_eq!(Ktx2Loader::default().extensions(), ["ktx2", "basis"]);
}

/// Exhaustive sweep (every byte, three different flips); run with `--ignored`.
#[test]
#[ignore = "slow exhaustive fuzz sweep"]
fn exhaustive_damage_sweep_of_ktx2_basis_files() {
    let loader = loader(BasisTarget::Rgba8, true);
    for file in [
        basis_file_to_ktx2(ETC, false).unwrap(),
        basis_file_to_ktx2(UASTC, true).unwrap(),
    ] {
        for i in 0..file.len() {
            for flip in [0x01u8, 0x80, 0xFF] {
                let mut broken = file.clone();
                broken[i] ^= flip;
                let _ = loader.load(&broken);
            }
        }
    }
}
