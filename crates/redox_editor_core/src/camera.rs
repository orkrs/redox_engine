use redox_math::{Aabb, Mat4, Quat, Vec3};

/// Editor fly/orbit camera stored as World resource.
#[derive(Clone, Debug)]
pub struct EditorCamera {
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub viewport_size: (u32, u32),
    pub speed: f32,
    /// Orbit pivot (updated on focus / selection).
    pub pivot: Vec3,
    /// If set, use orthographic projection with this half-extent (world units).
    pub ortho_half: Option<f32>,
}

impl Default for EditorCamera {
    fn default() -> Self {
        Self {
            eye: Vec3::new(0.0, 8.0, 12.0),
            yaw: 0.0,
            pitch: -0.35,
            fov_y: 60_f32.to_radians(),
            near: 0.05,
            far: 1000.0,
            viewport_size: (1280, 720),
            speed: 8.0,
            pivot: Vec3::ZERO,
            ortho_half: None,
        }
    }
}

impl EditorCamera {
    /// Top-down ortho looking at pivot.
    pub fn ortho_top(pivot: Vec3, half: f32) -> Self {
        Self {
            eye: pivot + Vec3::new(0.0, half * 2.0, 0.001),
            yaw: 0.0,
            pitch: -std::f32::consts::FRAC_PI_2 + 0.001,
            fov_y: 60_f32.to_radians(),
            near: 0.05,
            far: 2000.0,
            viewport_size: (640, 360),
            speed: 8.0,
            pivot,
            ortho_half: Some(half),
        }
    }

    /// Front ortho (−Z).
    pub fn ortho_front(pivot: Vec3, half: f32) -> Self {
        Self {
            eye: pivot + Vec3::new(0.0, 0.0, half * 2.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 60_f32.to_radians(),
            near: 0.05,
            far: 2000.0,
            viewport_size: (640, 360),
            speed: 8.0,
            pivot,
            ortho_half: Some(half),
        }
    }

    /// Right ortho (+X).
    pub fn ortho_right(pivot: Vec3, half: f32) -> Self {
        Self {
            eye: pivot + Vec3::new(half * 2.0, 0.0, 0.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            fov_y: 60_f32.to_radians(),
            near: 0.05,
            far: 2000.0,
            viewport_size: (640, 360),
            speed: 8.0,
            pivot,
            ortho_half: Some(half),
        }
    }

    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation() * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation() * Vec3::Y
    }

    pub fn view_matrix(&self) -> Mat4 {
        let forward = self.forward();
        let up = self.up();
        Mat4::look_at_rh(self.eye, self.eye + forward, up)
    }

    pub fn proj_matrix(&self) -> Mat4 {
        let aspect = if self.viewport_size.1 == 0 {
            1.0
        } else {
            self.viewport_size.0 as f32 / self.viewport_size.1 as f32
        };
        if let Some(half) = self.ortho_half {
            let h = half.max(0.5);
            let w = h * aspect;
            Mat4::orthographic_rh(-w, w, -h, h, self.near, self.far)
        } else {
            Mat4::perspective_rh(self.fov_y, aspect, self.near, self.far)
        }
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj_matrix() * self.view_matrix()
    }

    pub fn fly(&mut self, wish: Vec3, dt: f32, boost: bool) {
        let speed = self.speed * if boost { 4.0 } else { 1.0 };
        let move_dir = self.right() * wish.x + Vec3::Y * wish.y + self.forward() * wish.z;
        let len = move_dir.length();
        if len > 1e-6 {
            self.eye += move_dir / len * speed * dt;
        }
    }

    pub fn look(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx;
        self.pitch = (self.pitch - dy).clamp(-1.55, 1.55);
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        let dist = (self.eye - self.pivot).length().max(0.5);
        self.yaw -= dx;
        self.pitch = (self.pitch - dy).clamp(-1.55, 1.55);
        self.eye = self.pivot - self.forward() * dist;
    }

    pub fn adjust_speed(&mut self, scroll_y: f32) {
        self.speed = (self.speed * (1.0 + scroll_y * 0.1)).clamp(1.0, 80.0);
    }

    pub fn focus_on(&mut self, aabb: Aabb) {
        let center = aabb.center();
        let size = aabb.size().length();
        let dist = (size * 0.6).max(2.0) / (self.fov_y * 0.5).tan();
        let dir = (self.eye - center).normalize_or_zero();
        let dir = if dir.length_squared() < 1e-6 {
            Vec3::new(0.0, 0.3, 1.0).normalize()
        } else {
            dir
        };
        self.pivot = center;
        self.eye = center + dir * dist;
        self.ortho_half = None;
    }

    pub fn set_viewport_size(&mut self, w: u32, h: u32) {
        self.viewport_size = (w.max(1), h.max(1));
    }
}
