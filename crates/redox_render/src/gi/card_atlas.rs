//! GPU card atlas (GI-02 step 3): the captured faces of all static objects in three textures and
//! a per-object table that maps an object id to its face rectangles.
//!
//! Textures (all `2048²` by default, 64 MB together):
//! * `albedo`: `Rgba8Unorm`, `rgb` albedo, `a` emission / 16;
//! * `normal_depth`: `Rgba8Unorm`, `xyz` normal, `a` depth code (255 = empty);
//! * `radiance`: `Rgba16Float`, lit radiance seen from each texel, written by `gi_card_light`.
//!
//! The table holds one [`CardObjectGpu`] per object id (the id is the index in the slice given to
//! [`CardAtlas::set_objects`]); ids without cards have zero rectangles.

use bytemuck::{Pod, Zeroable};

use super::card_capture::{CardImage, ObjectCards};
use super::cards::{AtlasRect, CARD_FACES, ShelfPacker};

/// Edge of the square atlas textures in texels.
pub const CARD_ATLAS_SIZE: u32 = 2048;
/// Empty texels around every card rectangle, so bilinear reads do not cross into a neighbour.
const GUTTER: u32 = 2;
/// Bytes per texel across the three textures: albedo 4, normal/depth 4, radiance 8.
pub const CARD_TEXEL_BYTES: u64 = 4 + 4 + 8;

/// One object's cards, mirrored by `struct CardObjectGpu` in `shaders/gi_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct CardObjectGpu {
    /// World AABB minimum (xyz); w unused.
    pub aabb_min: [f32; 4],
    /// World AABB maximum (xyz); w unused.
    pub aabb_max: [f32; 4],
    /// Atlas rectangle `(x, y, w, h)` of each face in `CardFace::ALL` order; zero size = no card.
    pub rects: [[u32; 4]; CARD_FACES],
}

const _: () = assert!(std::mem::size_of::<CardObjectGpu>() == 128);

/// CPU result of packing: texel arrays for the three uploads, and the object table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackedCards {
    /// Atlas edge in texels.
    pub size: u32,
    /// `rgba` albedo texels, row-major.
    pub albedo: Vec<[u8; 4]>,
    /// `rgba` normal and depth texels, row-major.
    pub normal_depth: Vec<[u8; 4]>,
    /// One entry per object id up to the largest id that has a card.
    pub table: Vec<CardObjectGpu>,
    /// Faces that did not fit in the atlas (left without a card, so the trace falls back).
    pub dropped_faces: u32,
}

/// Packs the faces of `objects` (each with its id) into a `size`-square atlas.
///
/// Faces are placed in order with a shelf packer; a face that does not fit is dropped and counted.
pub fn pack_objects(objects: &[(ObjectCards, u32)], size: u32) -> PackedCards {
    let texels = (size as usize) * (size as usize);
    let mut packed = PackedCards {
        size,
        albedo: vec![[0; 4]; texels],
        normal_depth: vec![[0, 0, 0, 255]; texels],
        table: Vec::new(),
        dropped_faces: 0,
    };
    let mut packer = ShelfPacker::new(size, size);
    for (cards, id) in objects {
        let id = *id as usize;
        if packed.table.len() <= id {
            packed.table.resize(id + 1, empty_entry());
        }
        let mut entry = CardObjectGpu {
            aabb_min: [cards.aabb_min.x, cards.aabb_min.y, cards.aabb_min.z, 0.0],
            aabb_max: [cards.aabb_max.x, cards.aabb_max.y, cards.aabb_max.z, 0.0],
            rects: [[0; 4]; CARD_FACES],
        };
        for (f, image) in cards.faces.iter().enumerate() {
            match place_face(&mut packer, image) {
                Some(rect) => {
                    write_face(&mut packed, image, rect);
                    entry.rects[f] = [rect.x, rect.y, rect.w, rect.h];
                }
                None => packed.dropped_faces += 1,
            }
        }
        packed.table[id] = entry;
    }
    packed
}

fn empty_entry() -> CardObjectGpu {
    CardObjectGpu {
        aabb_min: [0.0; 4],
        aabb_max: [0.0; 4],
        rects: [[0; 4]; CARD_FACES],
    }
}

/// Allocates the face with its gutter; an empty face (no texels) gets no rectangle.
fn place_face(packer: &mut ShelfPacker, image: &CardImage) -> Option<AtlasRect> {
    if image.w == 0 || image.h == 0 {
        return None;
    }
    let outer = packer.alloc(image.w + 2 * GUTTER, image.h + 2 * GUTTER)?;
    Some(AtlasRect {
        x: outer.x + GUTTER,
        y: outer.y + GUTTER,
        w: image.w,
        h: image.h,
    })
}

fn write_face(packed: &mut PackedCards, image: &CardImage, rect: AtlasRect) {
    let size = packed.size as usize;
    for y in 0..image.h {
        for x in 0..image.w {
            let src = (x + y * image.w) as usize;
            let dst = (rect.x + x) as usize + (rect.y + y) as usize * size;
            packed.albedo[dst] = image.albedo[src];
            packed.normal_depth[dst] = image.normal[src];
        }
    }
}

/// GPU card atlas: the three textures, the object table buffer and the packing summary.
pub struct CardAtlas {
    /// Atlas edge in texels.
    pub size: u32,
    /// `Rgba8Unorm` albedo and emission scale.
    pub albedo_tex: wgpu::Texture,
    /// View of `albedo_tex`.
    pub albedo_view: wgpu::TextureView,
    /// `Rgba8Unorm` normal and depth code.
    pub normal_depth_tex: wgpu::Texture,
    /// View of `normal_depth_tex`.
    pub normal_depth_view: wgpu::TextureView,
    /// `Rgba16Float` lit radiance, written by the card light pass.
    pub radiance_tex: wgpu::Texture,
    /// View of `radiance_tex`.
    pub radiance_view: wgpu::TextureView,
    /// Storage buffer of [`CardObjectGpu`], one per object id.
    pub table: wgpu::Buffer,
    /// Number of table entries (largest object id + 1); zero when no cards are set.
    pub table_len: u32,
    /// Largest card edge in texels, bounds the light pass dispatch.
    pub max_edge: u32,
    /// Faces dropped by the packer in the last upload.
    pub dropped_faces: u32,
}

impl CardAtlas {
    /// Creates empty atlas textures of `size` texels per side.
    pub fn new(device: &wgpu::Device, size: u32) -> Self {
        let extent = wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        };
        let albedo_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gi_card_albedo"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let normal_depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gi_card_normal_depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let radiance_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gi_card_radiance"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_card_table"),
            size: std::mem::size_of::<CardObjectGpu>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            size,
            albedo_view: albedo_tex.create_view(&Default::default()),
            albedo_tex,
            normal_depth_view: normal_depth_tex.create_view(&Default::default()),
            normal_depth_tex,
            radiance_view: radiance_tex.create_view(&Default::default()),
            radiance_tex,
            table,
            table_len: 0,
            max_edge: 0,
            dropped_faces: 0,
        }
    }

    /// Replaces the cards with `objects` (captured faces and their object ids): packs the atlas,
    /// uploads the texels and the table. The radiance is not cleared; the light pass refreshes it.
    pub fn set_objects(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        objects: &[(ObjectCards, u32)],
    ) {
        let packed = pack_objects(objects, self.size);
        self.write_texels(queue, &packed);
        self.write_table(device, queue, &packed.table);
        self.table_len = packed.table.len() as u32;
        self.max_edge = max_face_edge(objects);
        self.dropped_faces = packed.dropped_faces;
    }

    /// Bytes of the three atlas textures (the `gi_cards` memory category).
    pub fn memory_bytes(&self) -> u64 {
        u64::from(self.size) * u64::from(self.size) * CARD_TEXEL_BYTES
    }

    fn write_texels(&self, queue: &wgpu::Queue, packed: &PackedCards) {
        let extent = wgpu::Extent3d {
            width: self.size,
            height: self.size,
            depth_or_array_layers: 1,
        };
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(self.size * 4),
            rows_per_image: Some(self.size),
        };
        for (texture, data) in [
            (&self.albedo_tex, &packed.albedo),
            (&self.normal_depth_tex, &packed.normal_depth),
        ] {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(data),
                layout,
                extent,
            );
        }
    }

    fn write_table(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, table: &[CardObjectGpu]) {
        let bytes = std::mem::size_of_val(table) as u64;
        if bytes > self.table.size() {
            self.table = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gi_card_table"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !table.is_empty() {
            queue.write_buffer(&self.table, 0, bytemuck::cast_slice(table));
        }
    }
}

/// Largest card edge over all faces of `objects`.
fn max_face_edge(objects: &[(ObjectCards, u32)]) -> u32 {
    objects
        .iter()
        .flat_map(|(cards, _)| cards.faces.iter())
        .map(|f| f.w.max(f.h))
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gi::card_capture::{CardMaterial, capture_object};
    use crate::mesh::{Mesh, Vertex};
    use redox_math::{Mat4, Vec3};

    fn unit_cube(texel: f32) -> ObjectCards {
        let v = |p: [f32; 3]| Vertex {
            position: p,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
        };
        let mesh = Mesh::new(vec![v([0.0, 0.0, 0.0]), v([1.0, 0.0, 0.0]), v([0.0, 1.0, 0.0])], vec![0, 1, 2]);
        let material = CardMaterial {
            albedo: Vec3::ONE,
            emissive: Vec3::ZERO,
        };
        capture_object(&mesh, &Mat4::IDENTITY, &material, texel)
    }

    #[test]
    fn card_object_gpu_is_128_bytes_and_zero_is_no_card() {
        assert_eq!(std::mem::size_of::<CardObjectGpu>(), 128);
        let packed = pack_objects(&[], 64);
        assert!(packed.table.is_empty());
        let packed = pack_objects(&[(unit_cube(0.25), 3)], 256);
        assert_eq!(packed.table.len(), 4);
        assert_eq!(packed.table[0].rects, [[0; 4]; CARD_FACES]);
        assert!(packed.table[3].rects.iter().any(|r| r[2] > 0));
    }

    #[test]
    fn packed_rectangles_stay_inside_the_atlas() {
        let packed = pack_objects(&[(unit_cube(0.25), 0)], 256);
        for rect in packed.table[0].rects {
            let [x, y, w, h] = rect;
            if w > 0 {
                assert!(x + w <= 256 && y + h <= 256);
            }
        }
        assert_eq!(packed.dropped_faces, 0);
    }
}
