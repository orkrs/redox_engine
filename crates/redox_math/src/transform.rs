use crate::{Mat4, Quat, Vec3};

/// World-space transform composed from the local [`Transform`] and ancestors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalTransform {
    /// Local-to-world matrix.
    pub matrix: Mat4,
}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }
}

impl GlobalTransform {
    /// The identity world transform.
    pub const IDENTITY: Self = Self {
        matrix: Mat4::IDENTITY,
    };

    /// Wraps an existing matrix.
    pub fn from_matrix(matrix: Mat4) -> Self {
        Self { matrix }
    }

    /// Translation part of the world matrix.
    pub fn translation(&self) -> Vec3 {
        self.matrix.to_scale_rotation_translation().2
    }
}

/// Единый компонент трансформации для всего движка
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Position relative to the parent (or the world when there is none).
    pub translation: Vec3,
    /// Orientation relative to the parent.
    pub rotation: Quat,
    /// Per-axis scale relative to the parent.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    /// Identity transform (origin, no rotation, unit scale).
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Creates a transform with only a translation.
    pub fn from_translation(t: Vec3) -> Self {
        Self {
            translation: t,
            ..Self::IDENTITY
        }
    }

    /// Creates a transform from translation and rotation.
    pub fn from_translation_rotation(t: Vec3, r: Quat) -> Self {
        Self {
            translation: t,
            rotation: r,
            ..Self::IDENTITY
        }
    }

    /// Вычисляет матрицу модели (Model Matrix) на основе параметров
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}
