// use redox_math::{Mat4, Transform};
use redox_math::Mat4;

pub struct System;
pub struct SystemStage;

// Компонент-дескриптор меша
#[derive(Debug, Clone)]
pub struct MeshHandle(pub usize);

// Компонент-дескриптор материала
#[derive(Debug, Clone)]
pub struct MaterialHandle(pub usize);

// Внутренняя структура для передачи данных на рендер
#[derive(Debug, Clone)]
pub struct RenderObject {
    pub model_matrix: Mat4,
    pub mesh_index: usize,
    pub material_index: usize,
}
