//! Physical shadow atlas — a large GPU texture storing rendered shadow pages.
//!
//! Pages are laid out in a grid within a single 2-D texture.  Each page
//! occupies `page_size × page_size` texels.  The atlas width in pages is
//! chosen to be a power of two for simple index→UV conversion.

use wgpu;

/// Format for the depth values stored in the atlas.
pub const VSM_ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// GPU shadow atlas.
pub struct ShadowAtlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// Number of pages along one axis of the atlas.
    pub pages_per_side: u32,
    /// Total atlas width/height in texels.
    pub size_px: u32,
    /// Size of a single page in texels.
    pub page_size_px: u32,
}

impl ShadowAtlas {
    /// Creates a new atlas that can hold `max_physical_pages` pages of
    /// `page_size_px × page_size_px` texels.
    ///
    /// If the requested capacity would exceed the wgpu maximum texture
    /// dimension (8192), the atlas is clamped to the largest square that
    /// fits.  The caller should query [`Self::max_pages`] to learn the
    /// actual capacity.
    pub fn new(device: &wgpu::Device, max_physical_pages: u32, page_size_px: u32) -> Self {
        const MAX_TEXTURE_DIM: u32 = 8192;
        let max_pps = MAX_TEXTURE_DIM / page_size_px.max(1);
        let ideal_pps = (max_physical_pages as f32).sqrt().ceil() as u32;
        let pages_per_side = ideal_pps.min(max_pps);
        let size_px = pages_per_side * page_size_px;

        if ideal_pps > max_pps {
            log::warn!(
                "[ShadowAtlas] Requested {} physical pages but atlas can only fit {} \
                 ({} x {} = {} px, clamped to max texture dim {}).",
                max_physical_pages,
                pages_per_side * pages_per_side,
                pages_per_side,
                page_size_px,
                size_px,
                MAX_TEXTURE_DIM,
            );
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vsm_shadow_atlas"),
            size: wgpu::Extent3d {
                width: size_px,
                height: size_px,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: VSM_ATLAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vsm_atlas_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            // R32Float atlases are sampled via `textureLoad` in WGSL, so the bind group
            // layout uses a NonFiltering sampler. Keep all filters Nearest to satisfy wgpu.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: None,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
            pages_per_side,
            size_px,
            page_size_px,
        }
    }

    /// Maximum number of physical pages this atlas can actually store.
    #[inline]
    pub fn max_pages(&self) -> u32 {
        self.pages_per_side * self.pages_per_side
    }

    /// Returns the pixel offset `(x, y)` of a physical page in the atlas.
    #[inline]
    pub fn page_offset(&self, phys_idx: u16) -> (u32, u32) {
        let col = (phys_idx as u32) % self.pages_per_side;
        let row = (phys_idx as u32) / self.pages_per_side;
        (col * self.page_size_px, row * self.page_size_px)
    }
}
