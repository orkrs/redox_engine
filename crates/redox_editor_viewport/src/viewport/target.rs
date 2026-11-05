//! Offscreen render target shared by the editor passes.

use wgpu::{Texture, TextureView};

/// Offscreen viewport for editor (Rgba8Unorm color + Depth32Float depth, sampled by egui)
pub struct EditorViewport {
    /// Color texture the viewport renders into.
    pub color_tex: Texture,
    /// View of the color texture.
    pub color_view: TextureView,
    /// Depth texture of the viewport.
    pub depth_tex: Texture,
    /// View of the depth texture.
    pub depth_view: TextureView,
    /// Size in pixels.
    pub size: (u32, u32),
}

impl EditorViewport {
    /// Creates a target of the given size.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("editor_viewport_color"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("editor_viewport_depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color_tex.create_view(&Default::default());
        let depth_view = depth_tex.create_view(&Default::default());
        Self {
            color_tex,
            color_view,
            depth_tex,
            depth_view,
            size: (w, h),
        }
    }

    /// Recreates the textures for a new size.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        *self = Self::new(device, width, height);
    }

    /// View of the color texture.
    pub fn color_view(&self) -> &TextureView {
        &self.color_view
    }
    /// View of the depth texture.
    pub fn depth_view(&self) -> &TextureView {
        &self.depth_view
    }
}
