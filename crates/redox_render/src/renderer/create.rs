//! Renderer creation for a window: instance, surface, adapter and device, then the shared construction.

use super::error::RendererError;
use super::*;

impl Renderer {
    /// Initialises the entire rendering context.
    ///
    /// Panics with the error text if the renderer cannot be created; use
    /// [`Self::try_new`] to handle the failure instead.
    ///
    /// This is an `async` function because `wgpu` adapter and device requests
    /// are asynchronous. Wrap with `pollster::block_on` when calling from
    /// synchronous code.
    pub async fn new(window: Arc<Window>, present_mode: wgpu::PresentMode) -> Self {
        Self::try_new(window, present_mode)
            .await
            .unwrap_or_else(|e| panic!("renderer creation failed: {e}"))
    }

    /// Initialises the entire rendering context, returning [`RendererError`]
    /// when the surface, GPU adapter or device cannot be created.
    ///
    /// This is an `async` function because `wgpu` adapter and device requests
    /// are asynchronous. Wrap with `pollster::block_on` when calling from
    /// synchronous code.
    pub async fn try_new(
        window: Arc<Window>,
        present_mode: wgpu::PresentMode,
    ) -> Result<Self, RendererError> {
        let size = window.inner_size();

        // --- Instance ---
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = wgpu::Backends::all();
        let instance = wgpu::Instance::new(instance_desc);

        // --- Surface ---
        let surface = instance
            .create_surface(window)
            .map_err(|e| RendererError::Surface(e.to_string()))?;

        // --- Adapter ---
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|_| RendererError::NoAdapter)?;

        log::info!("Using adapter: {:?}", adapter.get_info().name);

        // --- Device & Queue ---
        let mut required_features = wgpu::Features::empty();
        if adapter.features().contains(wgpu::Features::PRIMITIVE_INDEX) {
            required_features |= wgpu::Features::PRIMITIVE_INDEX;
        }
        // Rgba16Float MSAA×2 needs adapter-specific format features (WebGPU baseline is ×1/×4 only).
        if adapter
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }
        required_features |= adapter.features() & crate::profiler::PROFILER_FEATURES;
        // Block-compressed (BC1-BC7) textures from KTX2 and Basis files.
        required_features |= adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
        // Lets pipelines be cached on disk between runs (Vulkan).
        required_features |= adapter.features() & wgpu::Features::PIPELINE_CACHE;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("redox_device"),
                required_features,
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .map_err(|e| RendererError::Device(e.to_string()))?;

        // --- Surface configuration ---
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: Default::default(),
        };

        Ok(Self::from_shared_device(
            device,
            queue,
            &adapter,
            Some(surface),
            config,
        ))
    }
}
