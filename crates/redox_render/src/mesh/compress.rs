//! Vertex quantization: 44-byte [`super::Vertex`] → 16-byte [`CompressedVertex`].

use half::f16;
use redox_math::{Aabb, Vec3};

use super::Vertex;

/// GPU vertex layout: snorm16×4 pos (8) + snorm8×2 normal oct (2) +
/// snorm8×2 tangent oct (2) + f16×2 uv (4) = 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompressedVertex {
    /// Quantized local position (xyz) + tangent handedness sign in w (±1 as snorm16).
    pub position: [i16; 4],
    /// Octahedral-encoded normal.
    pub normal_oct: [i8; 2],
    /// Octahedral-encoded tangent direction.
    pub tangent_oct: [i8; 2],
    /// Half-precision UV (stored as raw f16 bits).
    pub uv: [u16; 2],
}

impl CompressedVertex {
    /// Vertex buffer layout of the compressed vertex format.
    pub fn buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Snorm16x4,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Snorm8x2,
                },
                wgpu::VertexAttribute {
                    offset: 10,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Snorm8x2,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float16x2,
                },
            ],
        }
    }
}

/// Position-only layout for depth-only passes (reads snorm16×4 at location 0).
pub fn compressed_position_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<CompressedVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            offset: 0,
            shader_location: 0,
            format: wgpu::VertexFormat::Snorm16x4,
        }],
    }
}

/// Returns `(min, extent)` vectors for shader dequantization.
pub fn bounds_vectors(aabb: &Aabb) -> ([f32; 4], [f32; 4]) {
    let min = aabb.min;
    let max = aabb.max;
    let ext = max - min;
    let ext = Vec3::new(ext.x.max(1e-6), ext.y.max(1e-6), ext.z.max(1e-6));
    ([min.x, min.y, min.z, 0.0], [ext.x, ext.y, ext.z, 0.0])
}

/// Quantize vertices relative to `aabb` into 16-byte records.
pub fn compress_vertices(vertices: &[Vertex], aabb: &Aabb) -> Vec<CompressedVertex> {
    let (min_arr, ext_arr) = bounds_vectors(aabb);
    let min = Vec3::from_array([min_arr[0], min_arr[1], min_arr[2]]);
    let ext = Vec3::from_array([ext_arr[0], ext_arr[1], ext_arr[2]]);

    vertices.iter().map(|v| compress_one(v, min, ext)).collect()
}

fn compress_one(v: &Vertex, min: Vec3, ext: Vec3) -> CompressedVertex {
    let p = Vec3::new(v.position[0], v.position[1], v.position[2]);
    let n = Vec3::new(v.normal[0], v.normal[1], v.normal[2]).normalize_or_zero();
    let t = Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2]).normalize_or_zero();
    let sign = if n.cross(t).dot(Vec3::new(0.0, 1.0, 0.0)) >= 0.0 {
        1.0_f32
    } else {
        -1.0_f32
    };

    let t_norm = (p - min) / ext;
    let snorm = |x: f32| -> i16 { (x.clamp(-1.0, 1.0) * 32767.0).round() as i16 };

    CompressedVertex {
        position: [
            snorm(t_norm.x * 2.0 - 1.0),
            snorm(t_norm.y * 2.0 - 1.0),
            snorm(t_norm.z * 2.0 - 1.0),
            snorm(sign),
        ],
        normal_oct: oct_encode_snorm8(n),
        tangent_oct: oct_encode_snorm8(t),
        uv: [
            f16::from_f32(v.uv[0]).to_bits(),
            f16::from_f32(v.uv[1]).to_bits(),
        ],
    }
}

fn oct_encode_snorm8(n: Vec3) -> [i8; 2] {
    let o = oct_encode_f32(n);
    [
        (o[0].clamp(-1.0, 1.0) * 127.0).round() as i8,
        (o[1].clamp(-1.0, 1.0) * 127.0).round() as i8,
    ]
}

fn oct_encode_f32(n: Vec3) -> [f32; 2] {
    let inv = (n.x.abs() + n.y.abs() + n.z.abs()).max(1e-6);
    let mut v = n / inv;
    if v.y < 0.0 {
        let ox = v.x;
        v.x = (1.0 - v.z.abs()) * if ox >= 0.0 { 1.0 } else { -1.0 };
        v.z = (1.0 - ox.abs()) * if v.z >= 0.0 { 1.0 } else { -1.0 };
    }
    [v.x, v.z]
}
