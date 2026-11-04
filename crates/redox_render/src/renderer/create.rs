//! Renderer creation for a window: instance, surface, adapter and device, then the shared construction.

use super::error::RendererError;
use super::*;

/// Upper bound requested for sampled textures per shader stage (reflection probes need 20+).
const SAMPLED_TEXTURES_CAP: u32 = 32;
/// Upper bound requested for storage buffers per shader stage.
const STORAGE_BUFFERS_CAP: u32 = 16;
/// Upper bound requested for storage textures per shader stage.
const STORAGE_TEXTURES_CAP: u32 = 8;

/// Device limits to request: the wgpu defaults, with three per-stage limits raised to
/// `min(adapter, cap)` but never lowered below the defaults.
pub fn required_device_limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    let defaults = wgpu::Limits::default();
    wgpu::Limits {
        max_sampled_textures_per_shader_stage: raised(
            adapter.max_sampled_textures_per_shader_stage,
            defaults.max_sampled_textures_per_shader_stage,
            SAMPLED_TEXTURES_CAP,
        ),
        max_storage_buffers_per_shader_stage: raised(
            adapter.max_storage_buffers_per_shader_stage,
            defaults.max_storage_buffers_per_shader_stage,
            STORAGE_BUFFERS_CAP,
        ),
        max_storage_textures_per_shader_stage: raised(
            adapter.max_storage_textures_per_shader_stage,
            defaults.max_storage_textures_per_shader_stage,
            STORAGE_TEXTURES_CAP,
        ),
        max_bind_groups: raised(
            adapter.max_bind_groups,
            defaults.max_bind_groups,
            crate::matgraph_cache::GRAPH_BIND_GROUPS,
        ),
        ..defaults
    }
}

fn raised(adapter_value: u32, default: u32, cap: u32) -> u32 {
    adapter_value.min(cap).max(default)
}

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
        // The VG traversal writes the instance slot into `first_instance` of its indirect draws.
        required_features |= adapter.features() & wgpu::Features::INDIRECT_FIRST_INSTANCE;
        let required_limits = if adapter.get_info().backend == wgpu::Backend::Vulkan {
            required_device_limits(&adapter.limits())
        } else {
            wgpu::Limits::default()
        };
        log::info!(
            "Device limits: sampled textures {}, storage buffers {}, storage textures {}",
            required_limits.max_sampled_textures_per_shader_stage,
            required_limits.max_storage_buffers_per_shader_stage,
            required_limits.max_storage_textures_per_shader_stage
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("redox_device"),
                required_features,
                required_limits,
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
