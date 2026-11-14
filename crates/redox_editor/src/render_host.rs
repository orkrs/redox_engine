//! Shared wgpu device + full engine [`Renderer`] offscreen into egui.
//!
//! # What works (real)
//! - One Device/Queue shared with egui-wgpu ([`SharedGpu`]).
//! - Full [`redox_render::Renderer`] via [`Renderer::from_shared_device`] (no window
//!   surface on the renderer — egui owns the swapchain).
//! - Engine frame path through [`Renderer::render_frame_into`]: VSM shadows, normal
//!   prepass, SSAO, opaque PBR, bloom, Sharp AA (MSAA resolve + SMAA + RCAS).
//! - Unit-cube + ground proxies uploaded into `GpuRegistry`.
//! - [`RenderHost::resize`] recreates the egui RTT; caller re-registers the texture.
//! - Quad layout: same Sharp-AA host path per pane (half-res pane RTTs, blit to
//!   viewport) when the engine renderer is available.
//!
//! # Stubs / not fully wired
//! - Virtual Geometry meshlet path: constructed with the engine but unused (no
//!   `VirtualMesh` entities in the editor world).
//! - GI / SDF shadow bricks: GPU objects exist; editor does not seed probes/bricks.
//! - IBL: loads project HDR / baked `.redox/ibl` when present; otherwise dummy + overlay.
//! - Mesh path: glTF via [`ViewportMeshCache`]; proxy cube only when asset missing.
//! - VG: meshlet bakes register on [`VGSystem`] via [`Self::register_meshlet_bake`].
//! - Shading modes report **unavailable** when the engine has no debug view (no cube hacks).

use redox_asset::{AssetLoader, Ktx2Loader};
use redox_editor_ui::MeshletBakeOutcome;
use redox_editor_viewport::{
    EditorCamera, EditorViewport, ViewportDrawItem, ViewportMeshCache, ViewportRuntimeSettings,
    ViewportShading, ViewportShadingPlan, ViewportStatsOverlay, apply_shading_plan,
    apply_viewport_runtime, mesh_wireframe_lines, quad_pane_cameras, screenshot_texture_to_png,
    vg_supported,
};
use redox_math::{Mat4, Transform, Vec3};
use redox_render::{
    Material, RenderObject, Renderer,
    mesh::primitive::create_cube,
    resource::ibl::IBLResource,
    resource::texture::Texture,
    virtual_geometry::{
        VGAssetId, VGInstanceHandle,
        cache as vg_cache,
    },
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use winit::window::Window;

const IBL_KTX2_FILES: [&str; 4] = [
    "environment.ktx2",
    "irradiance.ktx2",
    "prefiltered.ktx2",
    "brdf_lut.ktx2",
];

struct VgBakeRecord {
    asset_id: VGAssetId,
    instance: VGInstanceHandle,
    entity_id: Option<u32>,
}

/// Adapter features required by the engine HDR MSAA / Sharp AA path.
pub fn engine_required_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let mut required = wgpu::Features::empty();
    if adapter.features().contains(wgpu::Features::PRIMITIVE_INDEX) {
        required |= wgpu::Features::PRIMITIVE_INDEX;
    }
    if adapter
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
    {
        required |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    }
    required |= adapter.features() & redox_render::PROFILER_FEATURES;
    // Block-compressed textures and the on-disk pipeline cache, when the adapter has them.
    required |= adapter.features()
        & (wgpu::Features::TEXTURE_COMPRESSION_BC
            | wgpu::Features::PIPELINE_CACHE
            | wgpu::Features::INDIRECT_FIRST_INSTANCE);
    required
}

/// Shared GPU objects for egui + engine viewport.
///
/// wgpu 0.20 `Device`/`Queue` are not `Clone`, so the live device lives inside
/// [`RenderHost`]'s [`Renderer`]. This struct keeps the window swapchain only.
pub struct SharedGpu {
    pub adapter: wgpu::Adapter,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
}

pub struct SharedGpuBoot {
    pub shared: SharedGpu,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl SharedGpu {
    pub async fn request(window: Arc<Window>) -> SharedGpuBoot {
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = wgpu::Backends::all();
        let instance = wgpu::Instance::new(instance_desc);
        let surface = instance
            .create_surface(window.clone())
            .expect("editor surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("editor adapter");
        let required_features = engine_required_features(&adapter);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("redox_editor_shared_device"),
                required_features,
                required_limits: redox_render::renderer::required_device_limits(&adapter.limits()),
                ..Default::default()
            })
            .await
            .expect("editor device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: Default::default(),
        };
        surface.configure(&device, &config);
        SharedGpuBoot {
            shared: Self {
                adapter,
                surface,
                config,
            },
            device,
            queue,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(device, &self.config);
    }
}

/// Offscreen engine-path host: full [`Renderer`] → egui-sampled [`EditorViewport`].
pub struct RenderHost {
    pub viewport: EditorViewport,
    /// Full engine renderer (PBR + VSM + bloom + Sharp AA). `None` only if init failed.
    pub renderer: Option<Renderer>,
    cube_mesh: usize,
    default_mat: usize,
    selected_mat: usize,
    ground_mat: usize,
    width: u32,
    height: u32,
    /// Half-res pane RTTs for quad layout (Top/Front/Right/Persp).
    pane_views: Option<[EditorViewport; 4]>,
    object_scratch: Vec<RenderObject>,
    /// glTF / scene-ref uploads for real mesh draw.
    pub mesh_cache: ViewportMeshCache,
    /// Sky / GI / fog toggles (mirrors [`EditorSettings::viewport`]).
    pub runtime: ViewportRuntimeSettings,
    /// Latest overlay stats for the View menu strip.
    pub last_stats: ViewportStatsOverlay,
    /// Wireframe edge overlay for the current frame (Lit + wire mode).
    pub wireframe_overlay: Vec<(Vec3, Vec3, [f32; 4])>,
    vg_adapter_ok: bool,
    project_ibl_attempted: bool,
    vg_bakes: HashMap<u64, VgBakeRecord>,
    /// Rebuilds the passes of WGSL files edited while the editor runs (`shader-hot-reload`).
    #[cfg(feature = "shader-hot-reload")]
    shader_hot_reload: redox_render::renderer::ShaderHotReload,
    /// Material editor preview (MAT-10).
    pub material_preview: Option<crate::material_preview_host::MaterialPreviewRig>,
}

impl RenderHost {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: &wgpu::Adapter,
        w: u32,
        h: u32,
    ) -> Self {
        let width = w.max(1);
        let height = h.max(1);

        let viewport = EditorViewport::new(&device, width, height);
        let (vp_depth, vp_depth_view) = create_depth_tex(&device, "host_vp_depth", width, height);
        let mut viewport = viewport;
        viewport.depth_tex = vp_depth;
        viewport.depth_view = vp_depth_view;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: Default::default(),
        };

        let mut renderer = Renderer::from_shared_device(device, queue, adapter, None, config);
        renderer.set_vsm_enabled(true);
        renderer.set_smrt_quality(redox_render::shadow::SmrtQuality::Balanced);
        renderer.set_aa_sharp(true);
        renderer.set_ssao_enabled(true);
        renderer.bloom_enabled = true;
        renderer.set_exposure(1.1);
        renderer.set_rcas_sharpness(0.45);
        renderer.vg_use_vis_path = false;

        let cube_mesh = renderer.upload_mesh(&create_cube());
        let default_mat = renderer.add_material({
            let mut m = Material::solid(Vec3::new(0.72, 0.74, 0.78));
            m.roughness = 0.55;
            m.metallic = 0.05;
            m
        });
        let selected_mat = renderer.add_material({
            let mut m = Material::solid(Vec3::new(0.95, 0.75, 0.25));
            m.roughness = 0.4;
            m.metallic = 0.15;
            m
        });
        let ground_mat = renderer.add_material({
            let mut m = Material::solid(Vec3::new(0.22, 0.24, 0.28));
            m.roughness = 0.9;
            m
        });

        log::info!(
            "RenderHost: {width}x{height}, full Renderer offscreen (PBR+VSM+GTAO+bloom+Sharp AA); VG idle (no VirtualMesh entities)"
        );

        let pane_views = Some(make_pane_views(&renderer.device, width, height));

        let material_preview = Some(crate::material_preview_host::init_material_preview(&mut renderer));

        Self {
            viewport,
            renderer: Some(renderer),
            cube_mesh,
            default_mat,
            selected_mat,
            ground_mat,
            width,
            height,
            pane_views,
            object_scratch: Vec::new(),
            mesh_cache: ViewportMeshCache::new(),
            runtime: ViewportRuntimeSettings::default(),
            last_stats: ViewportStatsOverlay::default(),
            wireframe_overlay: Vec::new(),
            vg_adapter_ok: vg_supported(adapter),
            project_ibl_attempted: false,
            vg_bakes: HashMap::new(),
            #[cfg(feature = "shader-hot-reload")]
            shader_hot_reload: redox_render::renderer::ShaderHotReload::new(),
            material_preview,
        }
    }

    /// Registers a CPU meshlet bake on the viewport [`VGSystem`] (idempotent per cache key).
    pub fn register_meshlet_bake(
        &mut self,
        outcome: &MeshletBakeOutcome,
        transform: Mat4,
        entity_id: Option<u32>,
    ) -> String {
        let Some(renderer) = self.renderer.as_mut() else {
            return "gpu_register: FAIL no renderer".into();
        };
        let Some(vg) = renderer.vg_system.as_mut() else {
            return "gpu_register: FAIL no VGSystem".into();
        };
        if let Some(existing) = self.vg_bakes.get(&outcome.cache_key) {
            return format!("gpu_register: ok {}", existing.asset_id.0);
        }
        let Some(data) = vg_cache::load_cached(outcome.cache_key) else {
            return "gpu_register: FAIL cache missing".into();
        };
        let Some(id) = vg.register_asset(data, &renderer.queue) else {
            return "gpu_register: FAIL register_asset".into();
        };
        let transform_cols = transform.to_cols_array_2d();
        let Some(handle) = vg.spawn_instance(id, transform_cols) else {
            return "gpu_register: FAIL spawn_instance".into();
        };
        self.vg_bakes.insert(
            outcome.cache_key,
            VgBakeRecord {
                asset_id: id,
                instance: handle,
                entity_id,
            },
        );
        format!("gpu_register: ok {}", id.0)
    }

    fn sync_vg_instance_transforms(&mut self, draws: &[ViewportDrawItem]) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(vg) = renderer.vg_system.as_mut() else {
            return;
        };
        for record in self.vg_bakes.values() {
            let Some(entity_id) = record.entity_id else {
                continue;
            };
            let Some(item) = draws.iter().find(|d| d.entity_id == entity_id) else {
                continue;
            };
            let transform = item.transform.matrix().to_cols_array_2d();
            vg.update_instance_transform(record.instance, transform);
        }
    }

    fn ensure_project_ibl(&mut self, project_root: Option<&Path>, assets_root: Option<&Path>) {
        if self.project_ibl_attempted {
            return;
        }
        self.project_ibl_attempted = true;
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if !renderer.ibl_resource.is_dummy() {
            renderer.set_skybox(true);
            return;
        }
        if let Some(root) = project_root {
            let baked = root.join(".redox").join("ibl");
            if try_load_ibl_ktx2(renderer, &baked) {
                renderer.set_skybox(true);
                log::info!("RenderHost: IBL from {}", baked.display());
                return;
            }
        }
        if let Some(assets) = assets_root {
            for hdr in [assets.join("sky.hdr"), assets.join("skybox.hdr")] {
                if try_load_ibl_hdr(renderer, &hdr) {
                    log::info!("RenderHost: IBL from HDR {}", hdr.display());
                    return;
                }
            }
        }
        if let Some(root) = project_root {
            let assets = root.join("assets");
            for hdr in [
                assets.join("sky.hdr"),
                assets.join("skybox.hdr"),
                root.join("sky.hdr"),
            ] {
                if try_load_ibl_hdr(renderer, &hdr) {
                    log::info!("RenderHost: IBL from HDR {}", hdr.display());
                    return;
                }
            }
        }
        let dev = PathBuf::from("assets/foxrun/sky.hdr");
        if dev.is_file() && try_load_ibl_hdr(renderer, &dev) {
            log::info!("RenderHost: IBL from dev fallback {}", dev.display());
        }
    }

    /// Writes the current viewport colour target to `path` (blocking readback).
    pub fn screenshot_to_png(&self, path: &Path) -> Result<(), String> {
        let renderer = self.renderer.as_ref().ok_or("no renderer")?;
        screenshot_texture_to_png(
            &renderer.device,
            &renderer.queue,
            &self.viewport.color_tex,
            self.width,
            self.height,
            path,
        )
    }

    /// Refreshes [`Self::last_stats`] from editor timing inputs.
    pub fn record_stats(&mut self, fps: f32, cpu_ms: f32, plan: &ViewportShadingPlan) {
        let gpu_ms = self.renderer.as_ref().and_then(|r| {
            if r.gpu_profiler.is_enabled() && r.gpu_profiler.total_ms() > 0.0 {
                Some(r.gpu_profiler.total_ms())
            } else {
                None
            }
        });
        self.last_stats = ViewportStatsOverlay {
            fps,
            cpu_frame_ms: cpu_ms,
            gpu_frame_ms: gpu_ms,
            shading_line: plan.overlay_label(),
            missing_mesh_count: self.mesh_cache.missing_entities().len(),
            toggle_hints: Vec::new(),
        };
    }

    /// Logs compile errors for shaders saved since the last poll. Pipelines keep using the
    /// shaders loaded at startup: restart the editor to apply a passing edit.
    #[cfg(feature = "shader-hot-reload")]
    fn poll_shader_edits(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        // Validates edited shaders and rebuilds the passes of those that compile.
        let _ = self.shader_hot_reload.update(renderer);
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.renderer.as_ref().expect("RenderHost renderer").device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.renderer.as_ref().expect("RenderHost renderer").queue
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        self.viewport.color_view()
    }

    /// Recreate size-dependent targets. Caller must re-register the egui texture.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        self.viewport = EditorViewport::new(&renderer.device, width, height);
        let (vp_depth, vp_depth_view) =
            create_depth_tex(&renderer.device, "host_vp_depth", width, height);
        self.viewport.depth_tex = vp_depth;
        self.viewport.depth_view = vp_depth_view;

        self.pane_views = Some(make_pane_views(&renderer.device, width, height));
        renderer.resize(width, height);
    }

    /// Single-view engine frame into [`Self::viewport`].
    pub fn render(
        &mut self,
        camera: &EditorCamera,
        draws: &[ViewportDrawItem],
        selected_id: Option<u32>,
        shading: ViewportShading,
        assets_root: Option<&Path>,
        project_root: Option<&Path>,
    ) {
        #[cfg(feature = "shader-hot-reload")]
        self.poll_shader_edits();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        self.ensure_project_ibl(project_root, assets_root);
        self.sync_vg_instance_transforms(draws);
        let toggle_hints = apply_viewport_runtime(renderer, &self.runtime, self.vg_adapter_ok);
        apply_editor_camera(renderer, camera);
        let plan = ViewportShadingPlan::resolve(shading);
        apply_shading_plan(renderer, &plan);
        self.mesh_cache.clear_missing();
        self.wireframe_overlay.clear();
        self.object_scratch.clear();
        push_ground(&mut self.object_scratch, self.cube_mesh, self.ground_mat);
        push_draw_items(
            renderer,
            &mut self.mesh_cache,
            assets_root,
            &mut self.object_scratch,
            draws,
            selected_id,
            self.cube_mesh,
            self.default_mat,
            self.selected_mat,
            &plan,
            &mut self.wireframe_overlay,
        );

        let _ = renderer.render_frame_into(&self.object_scratch, self.viewport.color_view());
        clear_viewport_depth_for_overlay(renderer, &self.viewport);
        self.last_stats.toggle_hints = toggle_hints
            .overlay_lines()
            .into_iter()
            .map(str::to_string)
            .collect();
    }

    /// Quad layout: same engine path per pane (half-res), blit into viewport.
    pub fn render_quad(
        &mut self,
        camera: &EditorCamera,
        draws: &[ViewportDrawItem],
        selected_id: Option<u32>,
        shading: ViewportShading,
        assets_root: Option<&Path>,
        project_root: Option<&Path>,
    ) {
        let (tw, th) = (self.width.max(1), self.height.max(1));
        let hw = (tw / 2).max(1);
        let hh = (th / 2).max(1);
        let (top, front, right, persp) = quad_pane_cameras(camera, (tw, th));
        let cams = [top, front, right, persp];

        self.object_scratch.clear();
        push_ground(&mut self.object_scratch, self.cube_mesh, self.ground_mat);
        let plan = ViewportShadingPlan::resolve(shading);
        {
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            self.ensure_project_ibl(project_root, assets_root);
            let _ = apply_viewport_runtime(renderer, &self.runtime, self.vg_adapter_ok);
            self.mesh_cache.clear_missing();
            self.wireframe_overlay.clear();
            push_draw_items(
                renderer,
                &mut self.mesh_cache,
                assets_root,
                &mut self.object_scratch,
                draws,
                selected_id,
                self.cube_mesh,
                self.default_mat,
                self.selected_mat,
                &plan,
                &mut self.wireframe_overlay,
            );
        }
        let objects = self.object_scratch.clone();

        {
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            let Some(panes) = self.pane_views.as_ref() else {
                return;
            };

            let need_pane_resize = renderer.config.width != hw || renderer.config.height != hh;
            if need_pane_resize {
                renderer.resize(hw, hh);
            }

            apply_shading_plan(renderer, &plan);

            for (i, cam) in cams.iter().enumerate() {
                apply_editor_camera(renderer, cam);
                let _ = renderer.render_frame_into(&objects, panes[i].color_view());
            }

            if need_pane_resize || renderer.config.width != tw || renderer.config.height != th {
                renderer.resize(tw, th);
            }
        }

        let device = self.device();
        let queue = self.queue();
        let Some(panes) = self.pane_views.as_ref() else {
            return;
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("host_quad_blit"),
        });
        let origins = [(0u32, 0u32), (hw, 0), (0, hh), (hw, hh)];
        for (i, (ox, oy)) in origins.iter().enumerate() {
            let (pw, ph) = panes[i].size;
            let copy_w = pw
                .min(tw.saturating_sub(*ox))
                .min(if i % 2 == 0 { hw } else { tw - hw });
            let copy_h = ph
                .min(th.saturating_sub(*oy))
                .min(if i < 2 { hh } else { th - hh });
            if copy_w == 0 || copy_h == 0 {
                continue;
            }
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &panes[i].color_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.viewport.color_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: *ox,
                        y: *oy,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: copy_w,
                    height: copy_h,
                    depth_or_array_layers: 1,
                },
            );
        }
        {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("host_quad_depth_clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.viewport.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        queue.submit(Some(encoder.finish()));
    }
}

fn try_load_ibl_ktx2(renderer: &mut Renderer, dir: &Path) -> bool {
    let paths: Vec<PathBuf> = IBL_KTX2_FILES.iter().map(|name| dir.join(name)).collect();
    if !paths.iter().all(|p| p.is_file()) {
        return false;
    }
    let load = |path: &Path, label: &str| -> Option<Texture> {
        let bytes = std::fs::read(path).ok()?;
        let ktx = Ktx2Loader::default().load(&bytes).ok()?;
        Texture::from_ktx2(&renderer.device, &renderer.queue, &ktx, label)
    };
    let env = load(&paths[0], "ibl_env")?;
    let irr = load(&paths[1], "ibl_irr")?;
    let pre = load(&paths[2], "ibl_pre")?;
    let lut = load(&paths[3], "ibl_lut")?;
    match IBLResource::from_ktx2(env, irr, pre, lut) {
        Ok(resource) => {
            renderer.set_ibl_resource(resource);
            true
        }
        Err(err) => {
            log::warn!("RenderHost: IBL KTX2 load failed: {err}");
            false
        }
    }
}

fn try_load_ibl_hdr(renderer: &mut Renderer, hdr: &Path) -> bool {
    let Ok(bytes) = std::fs::read(hdr) else {
        return false;
    };
    let Ok(texture) = Texture::from_hdr_bytes(&renderer.device, &renderer.queue, &bytes, "sky") else {
        return false;
    };
    renderer.set_environment(&texture);
    renderer.set_skybox(true);
    true
}

fn make_pane_views(device: &wgpu::Device, width: u32, height: u32) -> [EditorViewport; 4] {
    let hw = (width.max(1) / 2).max(1);
    let hh = (height.max(1) / 2).max(1);
    [
        EditorViewport::new(device, hw, hh),
        EditorViewport::new(device, width.max(1) - hw, hh),
        EditorViewport::new(device, hw, height.max(1) - hh),
        EditorViewport::new(device, width.max(1) - hw, height.max(1) - hh),
    ]
}

fn create_depth_tex(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn apply_editor_camera(renderer: &mut Renderer, camera: &EditorCamera) {
    let view = camera.view_matrix();
    let proj = camera.proj_matrix();
    let vp = proj * view;
    renderer.camera_uniform.view = view.to_cols_array_2d();
    renderer.camera_uniform.proj = proj.to_cols_array_2d();
    renderer.camera_uniform.inv_view = view.inverse().to_cols_array_2d();
    renderer.camera_uniform.inv_proj = proj.inverse().to_cols_array_2d();
    renderer.camera_uniform.view_proj = vp.to_cols_array_2d();
    renderer.camera_uniform.camera_pos = [camera.eye.x, camera.eye.y, camera.eye.z, 1.0];
    renderer.camera_near = camera.near;
    renderer.camera_far = camera.far;
    renderer.update_camera_buffer();
}

fn push_ground(out: &mut Vec<RenderObject>, mesh: usize, mat: usize) {
    let model = Mat4::from_scale_rotation_translation(
        Vec3::new(64.0, 0.1, 64.0),
        redox_math::Quat::IDENTITY,
        Vec3::new(0.0, -0.05, 0.0),
    );
    let local = redox_math::Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5));
    out.push(RenderObject {
        id: out.len() as u32,
        shadow_class: redox_render::shadow::ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: model,
        color: [1.0, 1.0, 1.0, 1.0],
        mesh_index: mesh,
        material_index: mat,
        bounds: local.transform(model),
    });
}

fn push_draw_items(
    renderer: &mut Renderer,
    cache: &mut ViewportMeshCache,
    assets_root: Option<&Path>,
    out: &mut Vec<RenderObject>,
    draws: &[ViewportDrawItem],
    selected_id: Option<u32>,
    proxy_mesh: usize,
    default_mat: usize,
    selected_mat: usize,
    plan: &ViewportShadingPlan,
    wire_lines: &mut Vec<(Vec3, Vec3, [f32; 4])>,
) {
    let wire_color = [0.35, 0.95, 1.0, 1.0];
    for item in draws {
        let model = item.transform.matrix();
        let has_path = item.mesh_path.is_some() || item.mesh_ref.is_some();
        let resolved = if has_path {
            cache.resolve(renderer, assets_root, item)
        } else {
            None
        };
        let (mesh_index, material_index, bounds) = if let Some(r) = resolved {
            if plan.wireframe_overlay {
                wire_lines.extend(mesh_wireframe_lines(&r.cpu_mesh, model, wire_color));
            }
            (
                r.mesh_slot,
                if selected_id == Some(item.entity_id) {
                    selected_mat
                } else {
                    r.material_slot
                },
                r.local_bounds.transform(model),
            )
        } else {
            let mat = if selected_id == Some(item.entity_id) {
                selected_mat
            } else {
                default_mat
            };
            let local = redox_math::Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5));
            (proxy_mesh, mat, local.transform(model))
        };
        out.push(RenderObject {
            id: item.entity_id,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: model,
            color: [1.0, 1.0, 1.0, 1.0],
            mesh_index,
            material_index,
            bounds,
        });
    }
}

fn clear_viewport_depth_for_overlay(renderer: &Renderer, viewport: &EditorViewport) {
    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("host_vp_depth_clear"),
        });
    {
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("host_vp_depth_clear"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &viewport.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    renderer.queue.submit(Some(encoder.finish()));
}
