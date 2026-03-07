//! GPU context: owns the `wgpu` instance, device, queue, and surface.
//!
//! Also acts as a simple resource store for meshes, materials, and textures
//! in the MVP (these will move to a dedicated asset manager later).

use bytemuck;
use std::sync::Arc;
use winit::window::Window;

use crate::camera::CameraUniform;
use crate::light::LightUniform;
use crate::material::Material;
use crate::mesh::Mesh;
use crate::pass::forward::{ForwardPass, ModelUniform, create_depth_texture};
use crate::resource::buffer;
use crate::resource::texture::Texture;
use crate::systems::RenderObject;

/// GPU-side mesh data (vertex + index buffers).
pub struct GpuMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
}

/// The central rendering context.
///
/// Owns all `wgpu` state and provides methods to upload resources and render
/// a frame.
pub struct RenderContext {
    // --- wgpu core ---
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,

    // --- Render pass ---
    pub forward_pass: ForwardPass,

    // --- Per-frame uniforms ---
    pub camera_buffer: wgpu::Buffer,
    pub camera_bind_group: wgpu::BindGroup,
    pub camera_uniform: CameraUniform,

    pub light_buffer: wgpu::Buffer,
    pub light_bind_group: wgpu::BindGroup,

    // --- Depth ---
    #[allow(dead_code)]
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,

    // --- Resource storage (MVP) ---
    pub meshes: Vec<GpuMesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub texture_bind_groups: Vec<wgpu::BindGroup>,
    pub fallback_texture_bg: wgpu::BindGroup,

    /// Per-object model-matrix buffer (reused each frame).
    model_buffer: wgpu::Buffer,
    model_bind_group: wgpu::BindGroup,
}

impl RenderContext {
    /// Initialises the entire rendering context.
    ///
    /// This is an `async` function because `wgpu` adapter and device requests
    /// are asynchronous. Wrap with `pollster::block_on` when calling from
    /// synchronous code.
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        // --- Instance ---
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // --- Surface ---
        let surface = instance
            .create_surface(window)
            .expect("Failed to create surface");

        // --- Adapter ---
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find a suitable GPU adapter");

        log::info!("Using adapter: {:?}", adapter.get_info().name);

        // --- Device & Queue ---
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("redox_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .expect("Failed to create device");

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
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // --- Forward pass ---
        let forward_pass = ForwardPass::new(&device, surface_format);

        // --- Camera uniform ---
        let camera_uniform = CameraUniform::default();
        let camera_buffer = buffer::create_uniform_buffer(
            &device,
            "camera_uniform",
            bytemuck::bytes_of(&camera_uniform),
        );
        let camera_bind_group = forward_pass.create_camera_bind_group(&device, &camera_buffer);

        // --- Light uniform ---
        let light_uniform = LightUniform::default();
        let light_buffer = buffer::create_uniform_buffer(
            &device,
            "light_uniform",
            bytemuck::bytes_of(&light_uniform),
        );
        let light_bind_group = forward_pass.create_light_bind_group(&device, &light_buffer);

        // --- Depth ---
        let (depth_texture, depth_view) =
            create_depth_texture(&device, config.width, config.height);

        // --- Fallback texture ---
        let fallback_tex = Texture::white_1x1(&device, &queue);
        let fallback_texture_bg = forward_pass.create_texture_bind_group(&device, &fallback_tex);

        // --- Per-object model buffer (Storage) ---
        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("model_storage_buffer"),
            size: (64 * 10000) as u64, // Support up to 10,000 objects
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_bind_group = forward_pass.create_model_bind_group(&device, &model_buffer);

        Self {
            device,
            queue,
            surface,
            config,
            forward_pass,
            camera_buffer,
            camera_bind_group,
            camera_uniform,
            light_buffer,
            light_bind_group,
            depth_texture,
            depth_view,
            meshes: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
            texture_bind_groups: Vec::new(),
            fallback_texture_bg,
            model_buffer,
            model_bind_group,
        }
    }

    /// Reconfigure the surface and depth buffer after a window resize.
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width == 0 || new_height == 0 {
            return;
        }
        self.config.width = new_width;
        self.config.height = new_height;
        self.surface.configure(&self.device, &self.config);

        let (dt, dv) = create_depth_texture(&self.device, new_width, new_height);
        self.depth_texture = dt;
        self.depth_view = dv;

        log::info!("Resized to {}×{}", new_width, new_height);
    }

    // ----- Resource upload helpers -----

    /// Uploads a CPU [`Mesh`] to the GPU and returns its index (handle).
    pub fn upload_mesh(&mut self, mesh: &Mesh) -> usize {
        let vb = buffer::create_vertex_buffer(
            &self.device,
            "mesh_vb",
            bytemuck::cast_slice(&mesh.vertices),
        );
        let ib = buffer::create_index_buffer(
            &self.device,
            "mesh_ib",
            bytemuck::cast_slice(&mesh.indices),
        );
        let idx = self.meshes.len();
        self.meshes.push(GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count: mesh.index_count(),
        });
        idx
    }

    /// Stores a material and returns its index.
    pub fn add_material(&mut self, material: Material) -> usize {
        let idx = self.materials.len();
        self.materials.push(material);
        idx
    }

    /// Uploads a texture from raw bytes and returns its index.
    pub fn upload_texture(
        &mut self,
        bytes: &[u8],
        label: &str,
    ) -> Result<usize, image::ImageError> {
        let tex = Texture::from_bytes(&self.device, &self.queue, bytes, label)?;
        let bg = self
            .forward_pass
            .create_texture_bind_group(&self.device, &tex);
        let idx = self.textures.len();
        self.textures.push(tex);
        self.texture_bind_groups.push(bg);
        Ok(idx)
    }

    // ----- Rendering -----

    /// Renders a single frame given a list of [`RenderObject`]s.
    ///
    /// Call this after updating the camera and light uniforms.
    pub fn render_frame(&mut self, objects: &[RenderObject]) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("forward_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.08,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.forward_pass.pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.set_bind_group(1, &self.light_bind_group, &[]);

            // Update all model matrices at once
            let model_data: Vec<ModelUniform> = objects
                .iter()
                .map(|obj| ModelUniform {
                    model: obj.model_matrix.to_cols_array_2d(),
                })
                .collect();
            self.queue
                .write_buffer(&self.model_buffer, 0, bytemuck::cast_slice(&model_data));

            render_pass.set_bind_group(2, &self.model_bind_group, &[]);

            for (i, obj) in objects.iter().enumerate() {
                // Texture bind group
                let mut texture_bg = &self.fallback_texture_bg;
                if let Some(material) = self.materials.get(obj.material_index) {
                    if let Some(tex_idx) = material.texture_index {
                        if let Some(bg) = self.texture_bind_groups.get(tex_idx) {
                            texture_bg = bg;
                        }
                    }
                }
                render_pass.set_bind_group(3, texture_bg, &[]);

                if let Some(gpu_mesh) = self.meshes.get(obj.mesh_index) {
                    render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(
                        gpu_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    render_pass.draw_indexed(
                        0..gpu_mesh.index_count,
                        0,
                        (i as u32)..(i as u32 + 1),
                    );
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }

    /// Writes the current camera uniform to the GPU.
    pub fn update_camera_buffer(&self) {
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&self.camera_uniform),
        );
    }

    /// Writes a new light uniform to the GPU.
    pub fn update_light_buffer(&self, light: &LightUniform) {
        self.queue
            .write_buffer(&self.light_buffer, 0, bytemuck::bytes_of(light));
    }
}
