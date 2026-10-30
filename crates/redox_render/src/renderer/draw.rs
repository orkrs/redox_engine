//! Per-object draw recording (opaque and transparent).

use super::*;
use crate::matgraph_passes::GraphPass;

impl Renderer {
    /// Renders a single frame given a list of [`RenderObject`]s.
    ///
    pub fn begin_pass<'a>(&'a self, encoder: &'a mut wgpu::CommandEncoder) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("forward_pass"),
            color_attachments: &[Some(Self::hdr_color_attachment(
                &self.hdr_view,
                &self.hdr_msaa_view,
                self.msaa_sample_count,
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.22,
                    g: 0.26,
                    b: 0.34,
                    a: 1.0,
                }),
                HdrGeometryResolvePass::Forward,
                HdrGeometryResolvePass::Forward,
            ))],
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
            multiview_mask: None,
        })
    }

    /// Binds a material graph to the material slot `material_index` (MAT-02).
    ///
    /// On error the slot keeps its previous graph (or the default pipeline) and the error is
    /// logged. `params` gives parameter values by slot; missing entries use the slot defaults.
    pub fn set_material_graph(
        &mut self,
        material_index: usize,
        graph: &redox_matgraph::MaterialGraph,
        params: &[[f32; 4]],
    ) -> Result<(), crate::matgraph_cache::MaterialGraphError> {
        self.set_material_graph_with_textures(material_index, graph, params, &[])
    }

    /// Binds `graph` like [`Self::set_material_graph`] and binds the texture uploaded at
    /// registry index `texture_indices[i]` to `tex<i>` (MAT-03 step 1). A slot without an index,
    /// or with an index that names no texture, samples white.
    pub fn set_material_graph_with_textures(
        &mut self,
        material_index: usize,
        graph: &redox_matgraph::MaterialGraph,
        params: &[[f32; 4]],
        texture_indices: &[usize],
    ) -> Result<(), crate::matgraph_cache::MaterialGraphError> {
        let views: Vec<Option<&wgpu::TextureView>> = texture_indices
            .iter()
            .map(|&idx| self.registry.legacy_texture(idx).map(|t| &t.view))
            .collect();
        let result = self.matgraphs.set_material(
            &self.device,
            &self.queue,
            material_index,
            graph,
            params,
            &views,
        );
        if let Err(error) = &result {
            log::warn!("material {material_index}: graph not applied: {error}");
        }
        result.map(|_| ())
    }

    /// Selects the surface of material slot `material_index` from `instance.graph` (MAT-02
    /// step 5). `Some(id)` binds the graph registered under `id` in `graphs`; `None` returns the
    /// slot to the default pipelines. An unknown id is an error and the slot keeps its binding.
    pub fn set_material_instance(
        &mut self,
        material_index: usize,
        instance: &crate::material::MaterialInstance,
        graphs: &std::collections::HashMap<
            crate::material::MaterialGraphId,
            redox_matgraph::MaterialGraph,
        >,
        params: &[[f32; 4]],
    ) -> Result<(), crate::matgraph_cache::MaterialGraphError> {
        let Some(id) = instance.graph else {
            self.matgraphs.clear_material(material_index);
            return Ok(());
        };
        let Some(graph) = graphs.get(&id) else {
            let error = crate::matgraph_cache::MaterialGraphError::UnknownGraph(id);
            log::warn!("material {material_index}: {error}");
            return Err(error);
        };
        self.set_material_graph(material_index, graph, params)
    }

    /// Records the draw call for one object into `pass`.
    pub fn record_draw<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        debug_assert!(visibility.is_empty() || visibility.len() == objects.len());
        render_pass.set_pipeline(&self.pbr_pass.pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        let mut calls = 0u32;
        // True while the pass is bound to a graph variant pipeline (MAT-02).
        let mut on_graph = false;
        let same = |a: usize, b: usize| {
            objects[a].mesh_index == objects[b].mesh_index
                && objects[a].material_index == objects[b].material_index
        };
        for run in batching::runs(
            objects.len(),
            |i| visibility.is_empty() || visibility[i],
            same,
        ) {
            let obj = &objects[run.start as usize];
            // Graph-driven material: its variant pipeline and parameter block. Other slots keep
            // the default pipeline and nothing else changes for them.
            if let Some((pipeline, params_bg)) = self.matgraphs.binding(obj.material_index) {
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(
                    crate::matgraph_cache::GRAPH_PARAMS_GROUP,
                    params_bg,
                    &[],
                );
                on_graph = true;
            } else if on_graph {
                render_pass.set_pipeline(&self.pbr_pass.pipeline);
                on_graph = false;
            }
            // Material bind group (Group 2)
            let material_bg = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .unwrap_or_else(|| {
                    self.registry
                        .legacy_material_bind_group(0)
                        .expect("fallback material")
                });

            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, run);
                calls += 1;
            }
        }
        self.forward_draw_calls
            .store(calls, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns true when any visible object uses an alpha-blended material.
    ///
    /// Cheap pre-check so the transparent render pass is not opened for
    /// fully opaque frames.
    pub fn has_alpha_objects(&self, objects: &[RenderObject], visibility: &[bool]) -> bool {
        objects.iter().enumerate().any(|(i, obj)| {
            (visibility.is_empty() || visibility[i])
                && self
                    .material_blend_flags
                    .get(obj.material_index)
                    .copied()
                    .unwrap_or(BlendMode::Opaque)
                    != BlendMode::Opaque
        })
    }

    /// Draws alpha-blended objects into the HDR target with depth read-only.
    ///
    /// Objects are sorted back-to-front by distance from the camera
    /// (farthest drawn first) so alpha compositing order is correct.
    /// Instance indices stay GLOBAL — they index the model storage buffer,
    /// which holds all objects, not just the transparent ones.
    pub fn record_draw_transparent<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        let cam_pos = Vec3::new(
            self.camera_uniform.camera_pos[0],
            self.camera_uniform.camera_pos[1],
            self.camera_uniform.camera_pos[2],
        );

        // Collect visible alpha-blended object indices with their distance.
        let mut order: Vec<(usize, f32)> = objects
            .iter()
            .enumerate()
            .filter(|(i, obj)| {
                (visibility.is_empty() || visibility[*i])
                    && self
                        .material_blend_flags
                        .get(obj.material_index)
                        .copied()
                        .unwrap_or(BlendMode::Opaque)
                        == BlendMode::Alpha
            })
            .map(|(i, obj)| {
                let c = obj.bounds.center();
                (i, (c - cam_pos).length_squared())
            })
            .collect();

        // Farthest first (descending distance).
        order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if order.is_empty() {
            return;
        }

        render_pass.set_pipeline(&self.transparent_pass.pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        let mut on_graph = false;
        for (i, _) in order {
            let obj = &objects[i];
            // Graph-driven alpha material (MAT-02): its variant pipeline and parameter block.
            match self
                .matgraphs
                .pass_binding(obj.material_index, GraphPass::Alpha)
            {
                Some((pipeline, params)) => {
                    render_pass.set_pipeline(pipeline);
                    render_pass.set_bind_group(GraphPass::Alpha.params_group(), params, &[]);
                    on_graph = true;
                }
                None if on_graph => {
                    render_pass.set_pipeline(&self.transparent_pass.pipeline);
                    on_graph = false;
                }
                None => {}
            }
            let material_bg = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .unwrap_or_else(|| {
                    self.registry
                        .legacy_material_bind_group(0)
                        .expect("fallback material")
                });

            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
            }
        }
    }

    /// Draws additive objects (MAT-06) into the HDR target after the alpha pass.
    ///
    /// Additive blending is commutative, so no sorting is needed. Depth is
    /// tested against the opaque results but not written, as for alpha.
    pub fn record_draw_additive<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        let indices: Vec<usize> = objects
            .iter()
            .enumerate()
            .filter(|(i, obj)| {
                (visibility.is_empty() || visibility[*i])
                    && self
                        .material_blend_flags
                        .get(obj.material_index)
                        .copied()
                        .unwrap_or(BlendMode::Opaque)
                        == BlendMode::Additive
            })
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            return;
        }

        render_pass.set_pipeline(&self.transparent_pass.additive_pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        let mut on_graph = false;
        for i in indices {
            let obj = &objects[i];
            // Graph-driven additive material (MAT-02): its variant pipeline and parameter block.
            match self
                .matgraphs
                .pass_binding(obj.material_index, GraphPass::Additive)
            {
                Some((pipeline, params)) => {
                    render_pass.set_pipeline(pipeline);
                    render_pass.set_bind_group(GraphPass::Additive.params_group(), params, &[]);
                    on_graph = true;
                }
                None if on_graph => {
                    render_pass.set_pipeline(&self.transparent_pass.additive_pipeline);
                    on_graph = false;
                }
                None => {}
            }
            let Some(material_bg) = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .or_else(|| self.registry.legacy_material_bind_group(0))
            else {
                continue;
            };
            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
            }
        }
    }
}
