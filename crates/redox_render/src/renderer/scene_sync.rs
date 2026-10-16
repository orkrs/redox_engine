//! Per-frame CPU->GPU sync: camera, lights, models, clusters, virtual geometry.

use super::*;

impl Renderer {
    /// Writes the current camera uniform to the GPU.
    pub fn update_camera_buffer(&mut self) {
        let bytes = bytemuck::bytes_of(&self.camera_uniform).to_vec();
        self.staging_ring
            .write_immediate(&self.queue, &self.camera_buffer, 0, &bytes, false);
    }

    /// Returns the current light uniform.
    pub fn pbr_pass_light_uniform(&self) -> LightUniform {
        self.light_uniform
    }

    /// Writes a new light uniform to the GPU.
    ///
    /// When [`Self::vsm_enabled`] is true, forces `soft_shadows = 2` (VSM path)
    /// so the PBR shader never falls through to classic CSM.
    pub fn update_light_buffer(&mut self, light: &LightUniform) {
        let mut light = *light;
        if self.vsm_enabled {
            light.soft_shadows = 2;
        }
        self.light_uniform = light;
        let bytes = bytemuck::bytes_of(&light).to_vec();
        self.staging_ring
            .write_immediate(&self.queue, &self.light_buffer, 0, &bytes, false);
    }

    /// Writes the shader debug uniform (debug_viz_mode) to the GPU.
    pub fn update_shader_debug_buffer(&mut self, mode: ShaderDebugVizMode) {
        let mut u = ShaderDebugUniform::default();
        u.debug_viz_mode = mode as u32;
        u.frame_index = self.taa_frame_count as u32;
        u.ibl_enabled = u32::from(!self.ibl_resource.is_dummy());
        u.gtao_enabled = u32::from(self.ssao_enabled);
        let bytes = bytemuck::bytes_of(&u).to_vec();
        self.staging_ring
            .write_immediate(&self.queue, &self.shader_debug_buffer, 0, &bytes, false);
    }

    pub fn update_model_buffer(&mut self, objects: &[RenderObject]) {
        self.model_uniform_scratch.clear();
        self.model_uniform_scratch.reserve(objects.len());
        for obj in objects {
            let m3 = Mat3::from_cols(
                obj.model_matrix.x_axis.truncate(),
                obj.model_matrix.y_axis.truncate(),
                obj.model_matrix.z_axis.truncate(),
            );
            let nt = m3.inverse().transpose();
            let (bounds_min, bounds_extent) = self
                .registry
                .mesh_by_slot(obj.mesh_index)
                .map(|m| bounds_vectors(&m.local_bounds))
                .unwrap_or(([0.0; 4], [1.0, 1.0, 1.0, 0.0]));
            self.model_uniform_scratch.push(ModelUniform {
                model: obj.model_matrix.to_cols_array_2d(),
                normal_matrix: [
                    nt.x_axis.extend(0.0).to_array(),
                    nt.y_axis.extend(0.0).to_array(),
                    nt.z_axis.extend(0.0).to_array(),
                ],
                color: obj.color,
                bounds_min,
                bounds_extent,
            });
        }
        let bytes = bytemuck::cast_slice(&self.model_uniform_scratch).to_vec();
        self.staging_ring
            .write_immediate(&self.queue, &self.model_buffer, 0, &bytes, false);
    }

    /// Updates cluster light assignments. Call this when lights change or camera is updated.
    ///
    /// Must be called **after** `update_camera_buffer()` so that `camera_uniform.camera_pos`
    /// already holds the correct world-space camera position for this frame.
    pub fn update_cluster_lights(&mut self, lights: &[crate::light::PointLight], camera: &Camera) {
        self.current_point_lights = lights.to_vec();
        let cp = self.camera_uniform.camera_pos;
        let cam_pos = redox_math::Vec3::new(cp[0], cp[1], cp[2]);
        self.cluster_manager.update_clusters(
            lights,
            camera,
            cam_pos,
            Mat4::from_cols_array_2d(&self.camera_uniform.view_proj),
            &self.device,
            &self.queue,
        );
    }

    /// Returns cluster manager for GPU cluster buffer bindings (if needed).
    pub fn cluster_manager(&self) -> &ClusterManager {
        &self.cluster_manager
    }

    // ── Virtual Geometry ──────────────────────────────────────────────────────

    /// Initialise the Virtual Geometry subsystem with the given config.
    ///
    /// Call once after [`RenderContext::new`].  After this, `vg_system` is
    /// `Some` and VG assets / instances can be registered.
    pub fn init_virtual_geometry(&mut self, config: VGConfig) {
        // Use HDR format since VG objects are rendered into the HDR buffer in the forward pass
        let hdr_fmt = wgpu::TextureFormat::Rgba16Float;
        let depth_fmt = crate::pass::forward::DEPTH_FORMAT;
        let vg = VGSystem::new(
            &self.device,
            config,
            &self.camera_buffer,
            hdr_fmt,
            depth_fmt,
        );
        self.vg_system = Some(vg);
        self.rebuild_vg_bind_groups();
    }

    /// Register a mesh as a VG asset, returning its [`VGAssetId`].
    ///
    /// Requires [`init_virtual_geometry`] to have been called first.
    pub fn register_vg_mesh(
        &mut self,
        mesh: &crate::mesh::Mesh,
        material_index: u32,
    ) -> Option<VGAssetId> {
        self.vg_system
            .as_mut()?
            .register_mesh(mesh, material_index, &self.queue)
    }

    /// Prepare the VG system for this frame and synchronise transforms from ECS.
    ///
    /// Must be called each frame before [`render_frame`].
    pub fn prepare_vg_frame(&mut self, world: &mut redox_ecs::world::World) {
        let vg = match self.vg_system.as_mut() {
            Some(v) => v,
            None => return,
        };

        // Phase 1: collect VG entities via query (not all_entities scan).
        type Entity = redox_ecs::Entity;
        self.vg_sync_scratch.clear();
        for (entity, vm, transform) in world
            .query::<(Entity, &VirtualMesh, &Transform)>()
            .iter(world)
        {
            self.vg_sync_scratch.push((
                entity,
                transform.matrix().to_cols_array_2d(),
                vm.instance_handle,
                vm.asset_id,
            ));
        }

        // Phase 2: apply updates and spawn new instances, then write back handles
        let mut new_handles: Vec<(Entity, crate::virtual_geometry::VGInstanceHandle)> = Vec::new();
        for (entity, mat, handle_opt, asset_id) in &self.vg_sync_scratch {
            if let Some(handle) = *handle_opt {
                vg.update_instance_transform(handle, *mat);
            } else if let Some(handle) = vg.spawn_instance(*asset_id, *mat) {
                new_handles.push((*entity, handle));
            }
        }
        // Phase 3: write back newly assigned instance handles
        for (entity, handle) in new_handles {
            if let Some(vm_mut) = world.get_component_mut::<VirtualMesh>(entity) {
                vm_mut.instance_handle = Some(handle);
            }
        }

        // Run CPU culling and upload draw commands
        let vp = self.camera_uniform.view_proj;
        let cam_pos = redox_math::Vec3::from_array([
            self.camera_uniform.camera_pos[0],
            self.camera_uniform.camera_pos[1],
            self.camera_uniform.camera_pos[2],
        ]);
        // Recover vertical FOV from projection matrix (perspective RH [0,1]).
        let fov_y = 2.0 * (1.0 / self.camera_uniform.proj[1][1]).atan();
        vg.prepare_frame(&self.queue, &vp, cam_pos, fov_y);
    }
}
