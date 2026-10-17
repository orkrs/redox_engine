//! Cube maps, arrays and baked IBL through KTX2 on a real device (skipped without a GPU).

mod common;

use redox_asset::testing::{Ktx2Builder, Supercompression};
use redox_asset::{AssetLoader, Ktx2Loader, Ktx2Texture, TextureFormat};
use redox_render::resource::ibl::{IBLProcessor, IBLResource, read_texture_ktx2};
use redox_render::resource::texture::Texture;

fn load(builder: &Ktx2Builder) -> Ktx2Texture {
    Ktx2Loader::default()
        .load(&builder.build())
        .expect("valid ktx2")
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    common::device_with_bc(&common::adapter()?)
}

#[test]
fn cube_maps_arrays_and_cube_arrays_survive_a_gpu_round_trip() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let cases = [
        (
            Ktx2Builder::new(TextureFormat::Rgba8Unorm, 16, 16).cube(),
            wgpu::TextureViewDimension::Cube,
        ),
        (
            Ktx2Builder::new(TextureFormat::Rgba16Float, 8, 8)
                .cube()
                .layers(2),
            wgpu::TextureViewDimension::CubeArray,
        ),
        (
            Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 4).layers(3),
            wgpu::TextureViewDimension::D2Array,
        ),
        (
            Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8)
                .cube()
                .supercompression(Supercompression::Zstd),
            wgpu::TextureViewDimension::Cube,
        ),
    ];
    for (builder, dimension) in cases {
        let ktx = load(&builder);
        let texture = Texture::from_ktx2(&device, &queue, &ktx, "round_trip").expect("uploads");
        assert_eq!(texture.view_dimension, dimension);
        assert_eq!(texture.mip_levels as usize, ktx.levels.len());
        let back = read_texture_ktx2(&device, &queue, &texture.texture, ktx.is_cube())
            .expect("readable format");
        assert_eq!((back.layers, back.faces), (ktx.layers, ktx.faces));
        assert_eq!(
            back.levels, ktx.levels,
            "{dimension:?}: every mip, layer and face matches"
        );
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "validation error: {error:?}");
}

#[test]
fn bc7_cubes_upload_when_the_device_supports_bc() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let ktx = load(&Ktx2Builder::new(TextureFormat::Bc7Unorm, 16, 16).cube());
    let uploaded = Texture::from_ktx2(&device, &queue, &ktx, "bc7_cube");
    if device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        assert_eq!(
            uploaded.unwrap().view_dimension,
            wgpu::TextureViewDimension::Cube
        );
    } else {
        assert!(uploaded.is_none());
    }
}

#[test]
fn baked_ibl_round_trips_through_ktx2_and_lights_a_frame() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, 64) else {
        return;
    };
    let view = common::target(&renderer.device, 64);

    // A small sky gradient as the equirect source.
    let img = image::RgbaImage::from_fn(64, 32, |x, y| {
        image::Rgba([(x * 4) as u8, (y * 8) as u8, 200, 255])
    });
    let equirect = Texture::from_image(
        &renderer.device,
        &renderer.queue,
        &image::DynamicImage::ImageRgba8(img),
        "sky",
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    let processor = IBLProcessor::new(&renderer.device);
    let generated =
        IBLResource::from_equirect(&renderer.device, &renderer.queue, &processor, &equirect);
    assert!(!generated.is_dummy());

    // Bake to KTX2 bytes and load them back, as `redox_cook bake-ibl` plus a game would.
    let baked = generated
        .export_ktx2(&renderer.device, &renderer.queue)
        .expect("generated maps can be exported");
    assert!(
        baked.environment.is_cube() && baked.irradiance.is_cube() && baked.prefiltered.is_cube()
    );
    assert!(
        baked.prefiltered.levels.len() > 1,
        "roughness levels are kept as mips"
    );
    assert!(!baked.brdf_lut.is_cube());
    let reload = |tex: &Ktx2Texture| {
        let loaded = Ktx2Loader::default()
            .load(&tex.to_ktx2_bytes(true))
            .expect("baked file loads");
        assert_eq!(loaded.levels, tex.levels);
        loaded
    };
    let upload = |tex: &Ktx2Texture, label: &str| {
        Texture::from_ktx2(&renderer.device, &renderer.queue, tex, label).expect("uploads")
    };
    let from_files = IBLResource::from_ktx2(
        upload(&reload(&baked.environment), "env"),
        upload(&reload(&baked.irradiance), "irr"),
        upload(&reload(&baked.prefiltered), "pre"),
        upload(&reload(&baked.brdf_lut), "lut"),
    )
    .expect("shapes are right");
    assert!(!from_files.is_dummy());

    // The reloaded environment exports to the same bytes, and a frame renders with it.
    let again = from_files
        .export_ktx2(&renderer.device, &renderer.queue)
        .expect("exportable again");
    assert_eq!(again.irradiance.levels, baked.irradiance.levels);
    let scope = renderer
        .device
        .push_error_scope(wgpu::ErrorFilter::Validation);
    renderer.set_ibl_resource(from_files);
    renderer
        .render_frame_into(&[], &view)
        .expect("frame with baked IBL");
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "validation error: {error:?}");
}

#[test]
fn ibl_from_ktx2_rejects_wrong_shapes() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let flat_ktx = load(&Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8));
    let cube_ktx = load(&Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8).cube());
    let flat = |label| Texture::from_ktx2(&device, &queue, &flat_ktx, label).unwrap();
    let cube = |label| Texture::from_ktx2(&device, &queue, &cube_ktx, label).unwrap();
    assert!(IBLResource::from_ktx2(flat("e"), cube("i"), cube("p"), flat("lut")).is_err());
    assert!(IBLResource::from_ktx2(cube("e"), cube("i"), cube("p"), cube("lut")).is_err());
    assert!(IBLResource::from_ktx2(cube("e"), cube("i"), cube("p"), flat("lut")).is_ok());
}

#[test]
fn shared_layouts_bind_every_texture_shape() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(renderer) = common::renderer(&adapter, 32) else {
        return;
    };
    let scope = renderer
        .device
        .push_error_scope(wgpu::ErrorFilter::Validation);
    for builder in [
        Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8),
        Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8).layers(2),
        Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8).cube(),
        Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8)
            .cube()
            .layers(2),
    ] {
        let texture =
            Texture::from_ktx2(&renderer.device, &renderer.queue, &load(&builder), "shape")
                .unwrap();
        let _bind_group = renderer
            .shared_layouts
            .create_texture_bind_group(&renderer.device, &texture);
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "validation error: {error:?}");
}

#[test]
fn basis_files_load_in_the_best_format_for_the_device_and_upload() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(renderer) = common::renderer(&adapter, 32) else {
        return;
    };
    let expected = if renderer
        .device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        redox_asset::BasisTarget::Bc7
    } else {
        redox_asset::BasisTarget::Rgba8
    };
    assert_eq!(renderer.basis_target(), expected);

    let scope = renderer
        .device
        .push_error_scope(wgpu::ErrorFilter::Validation);
    for file in [
        &include_bytes!("../../redox_asset/tests/data/rust-logo-etc.basis")[..],
        &include_bytes!("../../redox_asset/tests/data/rust-logo-uastc.basis")[..],
    ] {
        let ktx = renderer.ktx2_loader().load(file).expect("transcodes");
        let want = if expected == redox_asset::BasisTarget::Bc7 {
            TextureFormat::Bc7Srgb
        } else {
            TextureFormat::Rgba8Srgb
        };
        assert_eq!(ktx.format, want);
        let texture = Texture::from_ktx2(&renderer.device, &renderer.queue, &ktx, "logo")
            .expect("the device takes the format it asked for");
        assert_eq!(texture.mip_levels, 7);
        let _ = renderer
            .shared_layouts
            .create_texture_bind_group(&renderer.device, &texture);
    }
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "validation error: {error:?}");
}
