//! Draw entry points of [`ViewportCubePass`].

use super::cube_pass::{LitUniform, ViewportCubePass, WireUniform};
use super::target::EditorViewport;
use crate::camera::EditorCamera;
use crate::picker::EntityPicker;
use crate::viewport_mode::QUAD_ORTHO_HALF;
use redox_math::Transform;

impl ViewportCubePass {
    /// `selected_id`: entity id to highlight, or None.
    /// `scissor`: optional pixel rect (x, y, w, h) within the offscreen target.
    /// `clear`: clear color/depth before drawing.
    ///
    /// Each entity is `write_buffer` + its own submit so uniforms are not clobbered
    /// (queue writes run before a submitted encoder, not interleaved with passes).
    pub fn render_entities(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        entities: &[(u32, Transform)],
        selected_id: Option<u32>,
        shading: crate::ViewportShading,
        scissor: Option<(u32, u32, u32, u32)>,
        clear: bool,
    ) {
        let view_proj = camera.proj_matrix() * camera.view_matrix();
        let light_dir = [0.35, -1.0, 0.25];
        let mode = shading.as_u32();

        if clear {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("editor_clear"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &viewport.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.09,
                                g: 0.10,
                                b: 0.12,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
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
                if let Some((x, y, w, h)) = scissor {
                    pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                    pass.set_scissor_rect(x, y, w, h);
                }
            }
            queue.submit(Some(encoder.finish()));
        }

        for (id, t) in entities {
            let model = t.matrix();
            let mvp = view_proj * model;

            if shading.is_wireframe() {
                let color = if selected_id == Some(*id) {
                    [1.0, 0.55, 0.12, 1.0]
                } else {
                    [0.85, 0.88, 0.92, 1.0]
                };
                let wu = WireUniform {
                    mvp: mvp.to_cols_array_2d(),
                    color,
                };
                queue.write_buffer(&self.wire_uniform_buffer, 0, bytemuck::bytes_of(&wu));
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("editor_wire_draw"),
                });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("editor_wire_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &viewport.color_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &viewport.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    if let Some((x, y, w, h)) = scissor {
                        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                        pass.set_scissor_rect(x, y, w, h);
                    }
                    pass.set_pipeline(&self.wire_pipeline);
                    pass.set_bind_group(0, &self.wire_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.wire_vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.wire_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.draw_indexed(0..self.wire_index_count, 0, 0..1);
                }
                queue.submit(Some(encoder.finish()));
                continue;
            }

            let u = LitUniform {
                mvp: mvp.to_cols_array_2d(),
                model: model.to_cols_array_2d(),
                light_dir,
                selected: if selected_id == Some(*id) { 1.0 } else { 0.0 },
                base_color: [0.62, 0.66, 0.72],
                mode,
            };
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("editor_lit_draw"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_lit_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &viewport.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &viewport.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if let Some((x, y, w, h)) = scissor {
                    pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                    pass.set_scissor_rect(x, y, w, h);
                }
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            queue.submit(Some(encoder.finish()));
        }
    }

    /// Quad layout: Top | Front / Right | Perspective into one texture.
    pub fn render_quad(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        perspective: &EditorCamera,
        entities: &[(u32, Transform)],
        selected_id: Option<u32>,
        shading: crate::ViewportShading,
    ) {
        let (tw, th) = viewport.size;
        let hw = tw / 2;
        let hh = th / 2;
        let pivot = perspective.pivot;
        let half = QUAD_ORTHO_HALF;
        let mut top = EditorCamera::ortho_top(pivot, half);
        top.set_viewport_size(hw, hh);
        let mut front = EditorCamera::ortho_front(pivot, half);
        front.set_viewport_size(hw, hh);
        let mut right = EditorCamera::ortho_right(pivot, half);
        right.set_viewport_size(hw, hh);
        let mut persp = perspective.clone();
        persp.ortho_half = None;
        persp.set_viewport_size(hw, hh);

        {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("quad_clear"),
            });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("quad_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &viewport.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.06,
                                g: 0.07,
                                b: 0.09,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
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
            queue.submit(Some(encoder.finish()));
        }

        let panes = [
            (&top, (0u32, 0u32, hw, hh)),
            (&front, (hw, 0, tw - hw, hh)),
            (&right, (0, hh, hw, th - hh)),
            (&persp, (hw, hh, tw - hw, th - hh)),
        ];
        for (cam, sc) in panes {
            {
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("quad_depth_clear"),
                });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("quad_depth_clear"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &viewport.color_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
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
                    pass.set_viewport(sc.0 as f32, sc.1 as f32, sc.2 as f32, sc.3 as f32, 0.0, 1.0);
                    pass.set_scissor_rect(sc.0, sc.1, sc.2, sc.3);
                }
                queue.submit(Some(encoder.finish()));
            }
            self.render_entities(
                device,
                queue,
                viewport,
                cam,
                entities,
                selected_id,
                shading,
                Some(sc),
                false,
            );
        }
    }

    /// Back-compat: old signature without selection.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        _time: f32,
    ) {
        self.render_entities(
            device,
            queue,
            viewport,
            camera,
            &[],
            None,
            crate::ViewportShading::Lit,
            None,
            true,
        );
    }

    pub fn render_pick_entities(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picker: &EntityPicker,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        entities: &[(u32, Transform)],
    ) {
        let view_proj = camera.proj_matrix() * camera.view_matrix();
        let id_view = picker.id_view.as_ref().expect("picker view");

        {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("editor_pick_clear"),
            });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_pick_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: id_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 4294967295.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
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
            queue.submit(Some(encoder.finish()));
        }

        for (entity_id, t) in entities {
            let mvp = view_proj * t.matrix();
            let mut data = [0u8; 80];
            data[0..64].copy_from_slice(bytemuck::bytes_of(&mvp.to_cols_array_2d()));
            data[64..68].copy_from_slice(&entity_id.to_ne_bytes());
            queue.write_buffer(&self.pick_uniform_buffer, 0, &data);
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("editor_pick_draw"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_pick_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: id_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &viewport.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&self.pick_pipeline);
                pass.set_bind_group(0, &self.pick_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            queue.submit(Some(encoder.finish()));
        }
    }

    pub fn render_pick(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picker: &EntityPicker,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        _time: f32,
        entity_id: u32,
    ) {
        self.render_pick_entities(
            device,
            queue,
            picker,
            viewport,
            camera,
            &[(entity_id, Transform::default())],
        );
    }

    pub fn render_ghost(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        ghost_transform: Transform,
    ) {
        self.render_entities(
            device,
            queue,
            viewport,
            camera,
            &[(u32::MAX - 1, ghost_transform)],
            None,
            crate::ViewportShading::Unlit,
            None,
            false,
        );
    }
}
