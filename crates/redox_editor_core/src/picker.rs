//! Entity picking — CPU raycast (reliable) + optional GPU ID buffer (safe polling).

use crate::camera::EditorCamera;
use crate::gizmo::screen_ray;
use crate::viewport_mode::{QUAD_ORTHO_HALF, QuadPane};
use redox_math::{Mat4, Transform, Vec2, Vec3};
use std::time::{Duration, Instant};

/// Max wait for a GPU ID-buffer read. Avoids `Maintain::Wait` deadlocks/crashes.
const GPU_PICK_TIMEOUT: Duration = Duration::from_millis(100);

/// Hardware Entity ID picker (R32Uint) — optional GPU path with safe timed polling.
pub struct EntityPicker {
    pub width: u32,
    pub height: u32,
    pub id_texture: Option<wgpu::Texture>,
    pub id_view: Option<wgpu::TextureView>,
    staging_buffer: Option<wgpu::Buffer>,
}

impl EntityPicker {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (tex, view, buf) = Self::create_resources(device, width, height);
        Self {
            width,
            height,
            id_texture: Some(tex),
            id_view: Some(view),
            staging_buffer: Some(buf),
        }
    }

    fn create_resources(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::Buffer) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("picker_id_texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let buf = Self::create_staging(device);
        (tex, view, buf)
    }

    fn create_staging(device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picker_staging_buffer"),
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (tex, view, buf) = Self::create_resources(device, width, height);
        self.width = width.max(1);
        self.height = height.max(1);
        self.id_texture = Some(tex);
        self.id_view = Some(view);
        self.staging_buffer = Some(buf);
    }

    /// Replace staging after a failed/abandoned map so the next read is usable.
    fn reset_staging(&mut self, device: &wgpu::Device) {
        self.staging_buffer = Some(Self::create_staging(device));
    }

    /// Read entity ID at pixel. Uses timed `Maintain::Poll` — never blocking `Wait`
    /// (which can deadlock/crash when the surface presents on the same device).
    ///
    /// Returns `None` on miss (`u32::MAX`), OOB, timeout, or map error.
    pub fn read_pixel_at(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        x: u32,
        y: u32,
    ) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let buf = self.staging_buffer.take()?;
        let Some(tex) = self.id_texture.as_ref() else {
            self.staging_buffer = Some(buf);
            return None;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("picker_read"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        // tex borrow ends here before we mutate self again

        let slice = buf.slice(..4);
        let (sender, receiver) = flume::bounded(1);
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });

        let deadline = Instant::now() + GPU_PICK_TIMEOUT;
        let map_ok = loop {
            // Safe polling only — never `Maintain::Wait`.
            device.poll(wgpu::Maintain::Poll);
            match receiver.try_recv() {
                Ok(Ok(())) => break true,
                Ok(Err(_)) => break false,
                Err(flume::TryRecvError::Disconnected) => break false,
                Err(flume::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        break false;
                    }
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
        };

        if !map_ok {
            drop(buf);
            self.reset_staging(device);
            return None;
        }

        let data = slice.get_mapped_range();
        let id = match data.get(0..4).and_then(|b| b.try_into().ok()) {
            Some(bytes) => u32::from_ne_bytes(bytes),
            None => {
                drop(data);
                buf.unmap();
                self.staging_buffer = Some(buf);
                return None;
            }
        };
        drop(data);
        buf.unmap();
        self.staging_buffer = Some(buf);
        if id == u32::MAX { None } else { Some(id) }
    }

    /// Non-blocking GPU pick: poll once after copy+map. Returns `None` if not ready
    /// or on miss. On pending/error, staging is replaced so the buffer is never stuck mapped.
    pub fn try_read_pixel_at(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        x: u32,
        y: u32,
    ) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let buf = self.staging_buffer.take()?;
        let Some(tex) = self.id_texture.as_ref() else {
            self.staging_buffer = Some(buf);
            return None;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("picker_try_read"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = buf.slice(..4);
        let (sender, receiver) = flume::bounded(1);
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        device.poll(wgpu::Maintain::Poll);

        match receiver.try_recv() {
            Ok(Ok(())) => {
                let data = slice.get_mapped_range();
                let id = match data.get(0..4).and_then(|b| b.try_into().ok()) {
                    Some(bytes) => u32::from_ne_bytes(bytes),
                    None => {
                        drop(data);
                        buf.unmap();
                        self.staging_buffer = Some(buf);
                        return None;
                    }
                };
                drop(data);
                buf.unmap();
                self.staging_buffer = Some(buf);
                if id == u32::MAX { None } else { Some(id) }
            }
            _ => {
                drop(buf);
                self.reset_staging(device);
                None
            }
        }
    }
}

/// Pick closest unit cube `[-0.5, 0.5]³` (matches [`crate::viewport::ViewportCubePass`] mesh) along a world ray.
pub fn pick_unit_cubes(entities: &[(u32, Transform)], origin: Vec3, dir: Vec3) -> Option<u32> {
    if dir.length_squared() < 1e-12 {
        return None;
    }
    let mut best_t = f32::MAX;
    let mut best_id = None;
    for &(id, ref t) in entities {
        let model = t.matrix();
        let Some(inv) = invert_affine(model) else {
            continue;
        };
        let local_o = inv.transform_point3(origin);
        let local_d = inv.transform_vector3(dir);
        if let Some(hit_t) = ray_aabb_t(local_o, local_d, Vec3::splat(-0.5), Vec3::splat(0.5)) {
            if hit_t >= 0.0 && hit_t < best_t {
                best_t = hit_t;
                best_id = Some(id);
            }
        }
    }
    best_id
}

/// Result of mapping a click in the full quad texture to a camera ray.
#[derive(Clone, Copy, Debug)]
pub struct QuadPickRay {
    pub pane: QuadPane,
    pub origin: Vec3,
    pub dir: Vec3,
    /// UV within the hit pane, in `[0, 1]²`.
    pub pane_uv: Vec2,
}

/// Resolve which quad pane a full-texture UV falls into, plus pane-local UV.
///
/// Layout matches [`crate::viewport::ViewportCubePass::render_quad`]:
/// ```text
/// Top   | Front
/// Right | Perspective
/// ```
pub fn quad_pane_at_uv(uv: Vec2, tex_size: (u32, u32)) -> (QuadPane, Vec2) {
    let (tw, th) = (tex_size.0.max(1), tex_size.1.max(1));
    let hw = tw / 2;
    let hh = th / 2;
    let x = (uv.x.clamp(0.0, 1.0) * tw as f32).min(tw as f32 - 1e-4);
    let y = (uv.y.clamp(0.0, 1.0) * th as f32).min(th as f32 - 1e-4);

    let (pane, ox, oy, pw, ph) = if (x as u32) < hw && (y as u32) < hh {
        (QuadPane::Top, 0u32, 0u32, hw, hh)
    } else if (x as u32) >= hw && (y as u32) < hh {
        (QuadPane::Front, hw, 0, tw - hw, hh)
    } else if (x as u32) < hw {
        (QuadPane::Right, 0, hh, hw, th - hh)
    } else {
        (QuadPane::Perspective, hw, hh, tw - hw, th - hh)
    };

    let pw = pw.max(1) as f32;
    let ph = ph.max(1) as f32;
    let pane_uv = Vec2::new(
        ((x - ox as f32) / pw).clamp(0.0, 1.0),
        ((y - oy as f32) / ph).clamp(0.0, 1.0),
    );
    (pane, pane_uv)
}

/// Build the same per-pane cameras as [`crate::viewport::ViewportCubePass::render_quad`].
pub fn quad_pane_cameras(
    perspective: &EditorCamera,
    tex_size: (u32, u32),
) -> (EditorCamera, EditorCamera, EditorCamera, EditorCamera) {
    let (tw, th) = (tex_size.0.max(1), tex_size.1.max(1));
    let hw = (tw / 2).max(1);
    let hh = (th / 2).max(1);
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
    (top, front, right, persp)
}

/// Map click UV in the full quad layout to the correct ortho/perspective camera ray.
///
/// Ready for the editor integrator: combine with [`pick_unit_cubes`] or GPU ID pick.
pub fn pick_in_quad_pane(
    uv: Vec2,
    perspective: &EditorCamera,
    tex_size: (u32, u32),
) -> QuadPickRay {
    let (pane, pane_uv) = quad_pane_at_uv(uv, tex_size);
    let (top, front, right, persp) = quad_pane_cameras(perspective, tex_size);
    let cam = match pane {
        QuadPane::Top => &top,
        QuadPane::Front => &front,
        QuadPane::Right => &right,
        QuadPane::Perspective => &persp,
    };
    let (origin, dir) = screen_ray(
        pane_uv,
        cam.view_matrix(),
        cam.proj_matrix(),
        cam.viewport_size,
    );
    QuadPickRay {
        pane,
        origin,
        dir,
        pane_uv,
    }
}

/// Convenience: quad UV → ray → closest unit-cube entity id (and which pane was hit).
pub fn pick_entity_in_quad_pane(
    entities: &[(u32, Transform)],
    uv: Vec2,
    perspective: &EditorCamera,
    tex_size: (u32, u32),
) -> Option<(u32, QuadPane)> {
    let ray = pick_in_quad_pane(uv, perspective, tex_size);
    pick_unit_cubes(entities, ray.origin, ray.dir).map(|id| (id, ray.pane))
}

fn invert_affine(m: Mat4) -> Option<Mat4> {
    let inv = m.inverse();
    if !inv.is_finite() { None } else { Some(inv) }
}

/// Ray/AABB intersection in parametric form `o + t*d`. Returns smallest `t >= 0`.
fn ray_aabb_t(o: Vec3, d: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut tmin = 0.0_f32;
    let mut tmax = f32::MAX;
    for i in 0..3 {
        let origin_i = o[i];
        let dir_i = d[i];
        let min_i = min[i];
        let max_i = max[i];
        if dir_i.abs() < 1e-8 {
            if origin_i < min_i || origin_i > max_i {
                return None;
            }
            continue;
        }
        let inv_d = 1.0 / dir_i;
        let mut t0 = (min_i - origin_i) * inv_d;
        let mut t1 = (max_i - origin_i) * inv_d;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        tmin = tmin.max(t0);
        tmax = tmax.min(t1);
        if tmin > tmax {
            return None;
        }
    }
    if tmax < 0.0 {
        None
    } else if tmin >= 0.0 {
        Some(tmin)
    } else {
        Some(tmax)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Transform;

    #[test]
    fn picks_front_cube() {
        let entities = [
            (1, Transform::from_translation(Vec3::new(0.0, 0.0, -2.0))),
            (2, Transform::from_translation(Vec3::new(0.0, 0.0, -5.0))),
        ];
        let origin = Vec3::ZERO;
        let dir = Vec3::new(0.0, 0.0, -1.0);
        assert_eq!(pick_unit_cubes(&entities, origin, dir), Some(1));
    }

    #[test]
    fn picks_nothing_on_zero_dir() {
        let entities = [(1, Transform::from_translation(Vec3::new(0.0, 0.0, -2.0)))];
        assert_eq!(pick_unit_cubes(&entities, Vec3::ZERO, Vec3::ZERO), None);
    }

    #[test]
    fn picks_misses_aside() {
        let entities = [(1, Transform::from_translation(Vec3::new(0.0, 0.0, -2.0)))];
        let origin = Vec3::new(10.0, 0.0, 0.0);
        let dir = Vec3::new(0.0, 0.0, -1.0);
        assert_eq!(pick_unit_cubes(&entities, origin, dir), None);
    }

    #[test]
    fn quad_pane_uv_corners() {
        let size = (200, 100);
        assert_eq!(quad_pane_at_uv(Vec2::new(0.1, 0.1), size).0, QuadPane::Top);
        assert_eq!(
            quad_pane_at_uv(Vec2::new(0.75, 0.1), size).0,
            QuadPane::Front
        );
        assert_eq!(
            quad_pane_at_uv(Vec2::new(0.1, 0.75), size).0,
            QuadPane::Right
        );
        assert_eq!(
            quad_pane_at_uv(Vec2::new(0.75, 0.75), size).0,
            QuadPane::Perspective
        );
    }

    #[test]
    fn pick_in_quad_pane_perspective_hits_cube() {
        let mut cam = EditorCamera::default();
        cam.eye = Vec3::new(0.0, 0.0, 5.0);
        cam.yaw = 0.0;
        cam.pitch = 0.0;
        cam.pivot = Vec3::ZERO;
        cam.set_viewport_size(200, 100);

        let entities = [(7, Transform::from_translation(Vec3::ZERO))];
        // Bottom-right pane center ≈ looking toward origin from +Z.
        let uv = Vec2::new(0.75, 0.75);
        let ray = pick_in_quad_pane(uv, &cam, (200, 100));
        assert_eq!(ray.pane, QuadPane::Perspective);
        assert!(ray.dir.length() > 0.5);
        assert_eq!(
            pick_entity_in_quad_pane(&entities, uv, &cam, (200, 100)),
            Some((7, QuadPane::Perspective))
        );
    }

    #[test]
    fn pick_in_quad_pane_top_has_downward_ray() {
        let mut cam = EditorCamera::default();
        cam.pivot = Vec3::ZERO;
        cam.set_viewport_size(200, 100);
        let ray = pick_in_quad_pane(Vec2::new(0.25, 0.25), &cam, (200, 100));
        assert_eq!(ray.pane, QuadPane::Top);
        // Top ortho looks roughly −Y.
        assert!(ray.dir.y < -0.5, "expected downward ray, got {:?}", ray.dir);
    }
}
