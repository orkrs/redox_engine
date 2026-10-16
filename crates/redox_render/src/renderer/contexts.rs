//! Lightweight pass contexts for multi-threaded command-buffer recording.

use redox_math::Mat4;

use crate::pass::normal::NormalPass;
use crate::pass::shadow::ShadowPass;
use crate::pass::vg_resolve::VgShadowPass;
use crate::registry::GpuRegistry;
use crate::systems::RenderObject;
use crate::virtual_geometry::runtime::VGSystem;

/// Clone-friendly handles for the depth/normal prepass.
pub struct PrepassContext<'a> {
    pub device: &'a wgpu::Device,
    pub normal_pass: &'a NormalPass,
    pub normal_bind_group: &'a wgpu::BindGroup,
    pub normal_view: &'a wgpu::TextureView,
    pub depth_view: &'a wgpu::TextureView,
    pub registry: &'a GpuRegistry,
}

/// Records the normal + depth prepass into a fresh command buffer.
pub fn record_normal_prepass(
    ctx: &PrepassContext<'_>,
    objects: &[RenderObject],
    vis: &[bool],
) -> wgpu::CommandBuffer {
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("prepass_encoder"),
        });

    let mut normal_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("normal_pass_parallel"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: ctx.normal_view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: ctx.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });

    normal_render_pass.set_pipeline(&ctx.normal_pass.pipeline);
    normal_render_pass.set_bind_group(0, ctx.normal_bind_group, &[]);
    for (i, obj) in objects.iter().enumerate() {
        if i >= vis.len() || !vis[i] {
            continue;
        }
        if let Some(gpu_mesh) = ctx.registry.mesh_by_slot(obj.mesh_index) {
            normal_render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
            normal_render_pass
                .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            normal_render_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
        }
    }
    drop(normal_render_pass);

    encoder.finish()
}

/// Frustum visibility scratch helper (shared by prepass scheduling).
pub fn compute_main_visibility(
    view_proj: &[[f32; 4]; 4],
    objects: &[RenderObject],
    out: &mut Vec<bool>,
) {
    let frustum = redox_math::Frustum::from_view_projection(Mat4::from_cols_array_2d(view_proj));
    out.clear();
    out.reserve(objects.len());
    for o in objects {
        out.push(frustum.intersects_aabb(&o.bounds));
    }
}

/// One point-light caster's atlas job (matrices already uploaded).
pub struct AtlasCasterJob {
    pub face_base: usize,
    pub tile_base: u32,
    pub face_vis: [Vec<bool>; 6],
}

pub struct AtlasContext<'a> {
    pub device: &'a wgpu::Device,
    pub shadow_pass: &'a ShadowPass,
    pub shadow_model_bind_group: &'a wgpu::BindGroup,
    pub face_matrix_bgs: &'a [wgpu::BindGroup],
    pub atlas_view: &'a wgpu::TextureView,
    pub vg_shadow_pass: &'a VgShadowPass,
    pub vg_shadow_instances_bg: &'a wgpu::BindGroup,
    pub registry: &'a GpuRegistry,
}

pub fn record_point_atlas_shadows(
    ctx: &AtlasContext<'_>,
    jobs: &[AtlasCasterJob],
    objects: &[RenderObject],
    vg_system: Option<&VGSystem>,
) -> wgpu::CommandBuffer {
    use crate::shadow::atlas_allocator::TILE_PX;

    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("shadow_atlas_encoder"),
        });
    if jobs.is_empty() {
        return encoder.finish();
    }

    let mut first = true;
    for job in jobs {
        let load_op = if first {
            first = false;
            wgpu::LoadOp::Clear(1.0)
        } else {
            wgpu::LoadOp::Load
        };
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("local_shadow_pass_point_cube"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: ctx.atlas_view,
                depth_ops: Some(wgpu::Operations {
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        shadow_pass.set_pipeline(&ctx.shadow_pass.pipeline);
        shadow_pass.set_bind_group(1, ctx.shadow_model_bind_group, &[]);

        for face_i in 0..6usize {
            let (col, row) =
                crate::shadow::atlas_allocator::tile_grid(job.tile_base + face_i as u32);
            let x = col * TILE_PX;
            let y = row * TILE_PX;
            shadow_pass.set_bind_group(0, &ctx.face_matrix_bgs[job.face_base + face_i], &[]);
            shadow_pass.set_viewport(x as f32, y as f32, TILE_PX as f32, TILE_PX as f32, 0.0, 1.0);
            shadow_pass.set_scissor_rect(x, y, TILE_PX, TILE_PX);
            for (i, obj) in objects.iter().enumerate() {
                if !job.face_vis[face_i][i] {
                    continue;
                }
                if let Some(gpu_mesh) = ctx.registry.mesh_by_slot(obj.mesh_index) {
                    shadow_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        gpu_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    shadow_pass.draw_indexed(
                        0..gpu_mesh.index_count,
                        0,
                        (i as u32)..(i as u32 + 1),
                    );
                }
            }
            if let Some(vg) = vg_system {
                if vg.has_content() {
                    vg.draw_shadow_indirect(
                        &mut shadow_pass,
                        ctx.vg_shadow_pass,
                        &ctx.face_matrix_bgs[job.face_base + face_i],
                        ctx.vg_shadow_instances_bg,
                    );
                }
            }
        }
    }
    encoder.finish()
}

pub struct VsmContext<'a> {
    pub device: &'a wgpu::Device,
    pub vsm_depth_pass: &'a crate::shadow::depth_pass::VsmDepthPass,
    pub vsm_pageinfo_bg: &'a wgpu::BindGroup,
    pub vsm_pages_views: &'a [wgpu::TextureView],
    pub registry: &'a GpuRegistry,
}

/// Draws every object that can cast into each dirty page. `page_vps[i]` is the view-projection
/// of `dirty[i]`; objects whose bounds fall outside its xy range are skipped (casters may sit
/// anywhere along the light ray, so depth is not tested).
pub fn record_vsm_pages(
    ctx: &VsmContext<'_>,
    dirty: &[(u32, u32, u32, u16)],
    page_vps: &[Mat4],
    objects: &[RenderObject],
) -> wgpu::CommandBuffer {
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vsm_pages_encoder"),
        });
    for (page_ord, &(_level, _px, _py, phys)) in dirty.iter().enumerate() {
        let mut page_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vsm_page_depth"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ctx.vsm_pages_views[phys as usize],
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        page_pass.set_pipeline(&ctx.vsm_depth_pass.pipeline);
        page_pass.set_bind_group(0, ctx.vsm_pageinfo_bg, &[(page_ord as u32) * 256]);
        let page_vp = page_vps.get(page_ord).copied();
        for (i, obj) in objects.iter().enumerate() {
            if page_vp.is_some_and(|vp| !aabb_overlaps_clip_xy(obj.bounds, vp)) {
                continue;
            }
            if let Some(gpu_mesh) = ctx.registry.mesh_by_slot(obj.mesh_index) {
                page_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                page_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                page_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
            }
        }
    }
    encoder.finish()
}

/// Whether the projected bounds touch the `[-1, 1]` xy square of `view_proj` (depth ignored).
fn aabb_overlaps_clip_xy(bounds: redox_math::Aabb, view_proj: Mat4) -> bool {
    let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
    for x in [bounds.min.x, bounds.max.x] {
        for y in [bounds.min.y, bounds.max.y] {
            for z in [bounds.min.z, bounds.max.z] {
                let clip = view_proj * redox_math::Vec4::new(x, y, z, 1.0);
                if clip.w.abs() < 1.0e-8 {
                    return true;
                }
                let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
                min_x = min_x.min(nx);
                max_x = max_x.max(nx);
                min_y = min_y.min(ny);
                max_y = max_y.max(ny);
            }
        }
    }
    !(max_x < -1.0 || min_x > 1.0 || max_y < -1.0 || min_y > 1.0)
}

#[cfg(test)]
mod page_cull_tests {
    use super::*;
    use redox_math::{Aabb, Vec3};

    fn unit_ortho() -> Mat4 {
        // Maps world x/y in [-2, 2] to NDC [-1, 1]; z untouched.
        Mat4::from_scale(Vec3::new(0.5, 0.5, 1.0))
    }

    #[test]
    fn object_inside_the_page_is_kept() {
        let b = Aabb {
            min: Vec3::new(-1.0, -1.0, -9.0),
            max: Vec3::new(1.0, 1.0, 9.0),
        };
        assert!(aabb_overlaps_clip_xy(b, unit_ortho()));
    }

    #[test]
    fn object_outside_the_page_is_culled_even_when_deep_in_z() {
        let b = Aabb {
            min: Vec3::new(5.0, -1.0, -100.0),
            max: Vec3::new(6.0, 1.0, 100.0),
        };
        assert!(!aabb_overlaps_clip_xy(b, unit_ortho()));
    }

    #[test]
    fn object_straddling_the_edge_is_kept() {
        let b = Aabb {
            min: Vec3::new(1.5, -0.5, 0.0),
            max: Vec3::new(3.5, 0.5, 1.0),
        };
        assert!(aabb_overlaps_clip_xy(b, unit_ortho()));
    }
}
