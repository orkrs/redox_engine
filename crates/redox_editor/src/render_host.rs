//! Shared wgpu device + full engine [`Renderer`] offscreen into egui.
//!
//! # What works (real)
//! - One Device/Queue shared with egui-wgpu ([`SharedGpu`]).
//! - Full [`redox_render::Renderer`] via [`Renderer::from_shared_device`] (no window
//!   surface on the renderer — egui owns the swapchain).
//! - Engine frame path through [`Renderer::render_frame_into`]: VSM shadows, normal
//!   prepass, SSAO, opaque PBR, bloom, Sharp AA (MSAA resolve + SMAA + RCAS).
//! - Unit-cube + ground proxies uploaded into [`GpuRegistry`].
//! - [`RenderHost::resize`] recreates the egui RTT; caller re-registers the texture.
//! - Quad layout: same Sharp-AA host path per pane (half-res pane RTTs, blit to
//!   viewport) when the engine renderer is available.
//!
//! # Stubs / not fully wired
//! - Virtual Geometry meshlet path: constructed with the engine but unused (no
//!   `VirtualMesh` entities in the editor world).
//! - DDGI / SDF shadow bricks: GPU objects exist; editor does not seed probes/bricks.
//! - IBL: dummy environment (no HDR load); skybox off.
//! - ViewportShading Unlit / Normals / Wireframe: Lit PBR only (selection tint).
//! - Streaming GpuRegistry assets beyond procedural cubes.

use redox_editor_core::{quad_pane_cameras, EditorCamera, EditorViewport, ViewportShading};
use redox_math::{Mat4, Transform, Vec3};
use redox_render::context::ShaderDebugVizMode;
use redox_render::{mesh::primitive::create_cube, Material, RenderObject, Renderer};
use std::sync::Arc;
use winit::window::Window;

/// Adapter features required by the engine HDR MSAA / Sharp AA path.
pub fn engine_required_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let mut required = wgpu::Features::empty();
    if adapter
        .features()
        .contains(wgpu::Features::SHADER_PRIMITIVE_INDEX)
    {
        required |= wgpu::Features::SHADER_PRIMITIVE_INDEX;
    }
    if adapter
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
    {
        required |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    }
    required |= adapter.features() & redox_render::PROFILER_FEATURES;
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
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(window.clone())
            .expect("editor surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("editor adapter");
        let required_features = engine_required_features(&adapter);
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("redox_editor_shared_device"),
                    required_features,
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
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
    /// Full engine renderer (PBR + CSM + bloom + Sharp AA). `None` only if init failed.
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
    /// Reports WGSL edits made while the editor runs (`shader-hot-reload` feature).
    #[cfg(feature = "shader-hot-reload")]
    shader_watcher: redox_render::shader::ShaderWatcher,
    #[cfg(feature = "shader-hot-reload")]
    shader_poll_frames: u32,
}

/// Frames between shader-directory polls (about twice a second at 60 fps).
#[cfg(feature = "shader-hot-reload")]
const SHADER_POLL_INTERVAL_FRAMES: u32 = 30;

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
            #[cfg(feature = "shader-hot-reload")]
            shader_watcher: redox_render::shader::ShaderWatcher::new(),
            #[cfg(feature = "shader-hot-reload")]
            shader_poll_frames: 0,
        }
    }

    /// Logs compile errors for shaders saved since the last poll. Pipelines keep using the
    /// shaders loaded at startup: restart the editor to apply a passing edit.
    #[cfg(feature = "shader-hot-reload")]
    fn poll_shader_edits(&mut self) {
        self.shader_poll_frames += 1;
        if self.shader_poll_frames < SHADER_POLL_INTERVAL_FRAMES {
            return;
        }
        self.shader_poll_frames = 0;
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        // Reports are logged by the watcher itself.
        let _ = self.shader_watcher.poll(&renderer.device);
    }

    pub fn device(&self) -> &wgpu::Device {
        &self
            .renderer
            .as_ref()
            .expect("RenderHost renderer")
            .device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self
            .renderer
            .as_ref()
            .expect("RenderHost renderer")
            .queue
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
        entities: &[(u32, Transform)],
        selected_id: Option<u32>,
        shading: ViewportShading,
    ) {
        #[cfg(feature = "shader-hot-reload")]
        self.poll_shader_edits();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        apply_editor_camera(renderer, camera);
        apply_shading(renderer, shading);
        self.object_scratch.clear();
        push_ground(&mut self.object_scratch, self.cube_mesh, self.ground_mat);
        push_entities(
            &mut self.object_scratch,
            entities,
            selected_id,
            self.cube_mesh,
            self.default_mat,
            self.selected_mat,
        );

        let _ = renderer.render_frame_into(&self.object_scratch, self.viewport.color_view());
        clear_viewport_depth_for_overlay(renderer, &self.viewport);
    }

    /// Quad layout: same engine path per pane (half-res), blit into viewport.
    pub fn render_quad(
        &mut self,
        camera: &EditorCamera,
        entities: &[(u32, Transform)],
        selected_id: Option<u32>,
        shading: ViewportShading,
    ) {
        let (tw, th) = (self.width.max(1), self.height.max(1));
        let hw = (tw / 2).max(1);
        let hh = (th / 2).max(1);
        let (top, front, right, persp) = quad_pane_cameras(camera, (tw, th));
        let cams = [top, front, right, persp];

        self.object_scratch.clear();
        push_ground(&mut self.object_scratch, self.cube_mesh, self.ground_mat);
        push_entities(
            &mut self.object_scratch,
            entities,
            selected_id,
            self.cube_mesh,
            self.default_mat,
            self.selected_mat,
        );
        let objects = self.object_scratch.clone();

        {
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            let Some(panes) = self.pane_views.as_ref() else {
                return;
            };

            let need_pane_resize =
                renderer.config.width != hw || renderer.config.height != hh;
            if need_pane_resize {
                renderer.resize(hw, hh);
            }

            apply_shading(renderer, shading);

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
            let copy_w = pw.min(tw.saturating_sub(*ox)).min(if i % 2 == 0 { hw } else { tw - hw });
            let copy_h = ph.min(th.saturating_sub(*oy)).min(if i < 2 { hh } else { th - hh });
            if copy_w == 0 || copy_h == 0 {
                continue;
            }
            encoder.copy_texture_to_texture(
                wgpu::ImageCopyTexture {
                    texture: &panes[i].color_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyTexture {
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
            });
        }
        queue.submit(Some(encoder.finish()));
    }
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

fn apply_shading(renderer: &mut Renderer, shading: ViewportShading) {
    let mode = match shading {
        ViewportShading::Lit | ViewportShading::Unlit | ViewportShading::Wireframe => {
            ShaderDebugVizMode::None
        }
        ViewportShading::Normals => ShaderDebugVizMode::ShadowSources,
    };
    renderer.update_shader_debug_buffer(mode);
}

fn push_ground(out: &mut Vec<RenderObject>, mesh: usize, mat: usize) {
    let model = Mat4::from_scale_rotation_translation(
        Vec3::new(64.0, 0.1, 64.0),
        redox_math::Quat::IDENTITY,
        Vec3::new(0.0, -0.05, 0.0),
    );
    let local = redox_math::Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5));
    out.push(RenderObject {
        model_matrix: model,
        color: [1.0, 1.0, 1.0, 1.0],
        mesh_index: mesh,
        material_index: mat,
        bounds: local.transform(model),
    });
}

fn push_entities(
    out: &mut Vec<RenderObject>,
    entities: &[(u32, Transform)],
    selected_id: Option<u32>,
    mesh: usize,
    default_mat: usize,
    selected_mat: usize,
) {
    for (id, t) in entities {
        let model = t.matrix();
        let mat = if selected_id == Some(*id) {
            selected_mat
        } else {
            default_mat
        };
        let local = redox_math::Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5));
        out.push(RenderObject {
            model_matrix: model,
            color: [1.0, 1.0, 1.0, 1.0],
            mesh_index: mesh,
            material_index: mat,
            bounds: local.transform(model),
        });
    }
}

fn clear_viewport_depth_for_overlay(renderer: &Renderer, viewport: &EditorViewport) {
    let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
        });
    }
    renderer.queue.submit(Some(encoder.finish()));
}
