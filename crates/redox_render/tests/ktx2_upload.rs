//! KTX2 textures uploaded to a real device (skipped when no adapter exists).

use redox_asset::{Ktx2Texture, TextureFormat};
use redox_render::resource::texture::Texture;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    let features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        ..Default::default()
    }))
    .ok()
}

fn ktx(format: TextureFormat, w: u32, h: u32) -> Ktx2Texture {
    let mut levels = Vec::new();
    let (mut lw, mut lh) = (w, h);
    loop {
        levels.push(vec![0x7f; format.level_size(lw, lh)]);
        if lw == 1 && lh == 1 {
            break;
        }
        lw = (lw / 2).max(1);
        lh = (lh / 2).max(1);
    }
    Ktx2Texture {
        width: w,
        height: h,
        format,
        levels,
    }
}

#[test]
fn uploads_rgba8_and_bc7_mip_chains_without_validation_errors() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let rgba = Texture::from_ktx2(
        &device,
        &queue,
        &ktx(TextureFormat::Rgba8Srgb, 13, 7),
        "rgba",
    )
    .expect("rgba8 always uploads");
    assert_eq!(rgba.mip_levels, 4);
    let bc = Texture::from_ktx2(&device, &queue, &ktx(TextureFormat::Bc7Unorm, 12, 8), "bc7");
    let odd = Texture::from_ktx2(&device, &queue, &ktx(TextureFormat::Bc7Unorm, 10, 6), "odd");
    assert!(
        odd.is_none(),
        "sizes that are not block multiples are refused"
    );
    if device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    {
        assert_eq!(bc.expect("bc7 uploads with the feature").mip_levels, 4);
    } else {
        assert!(bc.is_none());
    }
    queue.submit([]);
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "validation error: {error:?}");
}
