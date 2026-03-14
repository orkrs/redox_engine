//! Core mesh types: vertex layout and mesh data.

pub mod loader;
pub mod primitive;

use bytemuck::{Pod, Zeroable};
use redox_math::{Aabb, Vec3};

/// A single vertex with position, normal, and texture coordinates.
///
/// The layout is `#[repr(C)]` to ensure a predictable memory layout when
/// uploaded to the GPU via `bytemuck`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// World-space position.
    pub position: [f32; 3],
    /// Surface normal (should be normalised).
    pub normal: [f32; 3],
    /// Texture coordinates (UV).
    pub uv: [f32; 2],
    /// Surface tangent (for normal mapping).
    pub tangent: [f32; 3],
}

impl Vertex {
    /// Returns the `wgpu::VertexBufferLayout` describing this vertex type.
    pub fn buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // position
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // normal
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // uv
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 6]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // tangent
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

/// CPU-side mesh data (vertices + indices) with an optional bounding box.
///
/// After creation the mesh is purely on the CPU. Call
/// [`RenderContext::upload_mesh`] to create GPU buffers.
#[derive(Clone, Debug)]
pub struct Mesh {
    /// Vertex data.
    pub vertices: Vec<Vertex>,
    /// Triangle indices (three per face).
    pub indices: Vec<u32>,
    /// Axis-aligned bounding box computed from the vertices.
    pub aabb: Aabb,
}

impl Mesh {
    /// Creates a new mesh and computes its AABB from the vertex positions.
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        let mut aabb = Aabb::empty();
        for v in &vertices {
            aabb = aabb.expand(Vec3::new(v.position[0], v.position[1], v.position[2]));
        }
        Self {
            vertices,
            indices,
            aabb,
        }
    }

    /// Returns the number of indices (equal to 3 × number of triangles).
    #[inline]
    pub fn index_count(&self) -> u32 {
        self.indices.len() as u32
    }
}
