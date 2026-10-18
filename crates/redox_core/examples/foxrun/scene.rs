//! Everything drawn in Foxfire: the glade, the forest, gems, wisps and the fox.

use redox_anim::Pose;
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::light::{DirectionalLight, LightScene, PointLight};
use redox_render::material::Material;
use redox_render::mesh::primitive;
use redox_render::mesh::{Mesh, Vertex};
use redox_render::{RenderObject, Renderer, SkinnedMeshBinding};

use crate::fox::{FoxAssets, MODEL_SCALE};
use crate::world::{ARENA_RADIUS, World};

/// Trunk radius per unit of tree size, in metres.
pub const TRUNK: f32 = 0.28;
/// Crown radius per unit of tree size, in metres.
pub const CROWN: f32 = 1.25;
/// Gems lit by point lights at the same time.
const GEM_LIGHTS: usize = 8;
/// Gem lights are at full strength within this distance of the fox and fade out by `GEM_LIGHT_FADE_END`,
/// so a light never pops in when it joins the nearest few.
const GEM_LIGHT_FADE_START: f32 = 10.0;
const GEM_LIGHT_FADE_END: f32 = 22.0;
/// Decorative trees on the rim of the glade.
const RIM_TREES: usize = 90;

/// GPU resources and the fox's skinned mesh.
pub struct Scene {
    sphere: usize,
    ground: usize,
    grass: usize,
    trunk: usize,
    foliage: [usize; 4],
    rock: usize,
    gems: [usize; 3],
    wisp: usize,
    fox_material: usize,
    pub fox: SkinnedMeshBinding,
}

/// What one frame needs besides the camera.
pub struct Frame {
    pub objects: Vec<RenderObject>,
    pub ids: Vec<u64>,
    pub lights: LightScene,
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

fn unit(x: u32) -> f32 {
    (hash(x) & 0xFFFF) as f32 / 65535.0
}

/// The engine's sphere faces inward for back-face culling; reverse every triangle.
fn outward(mut mesh: Mesh) -> Mesh {
    for triangle in mesh.indices.chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
    mesh
}

/// A big flat square with UVs that repeat every `tile` metres.
fn ground_mesh(half: f32, tile: f32) -> Mesh {
    let corner = |x: f32, z: f32| Vertex {
        position: [x, 0.0, z],
        normal: [0.0, 1.0, 0.0],
        uv: [x / tile, z / tile],
        tangent: [1.0, 0.0, 0.0],
    };
    Mesh {
        vertices: vec![
            corner(-half, -half),
            corner(half, -half),
            corner(half, half),
            corner(-half, half),
        ],
        indices: vec![0, 3, 2, 0, 2, 1],
        aabb: Aabb::from_center_size(Vec3::ZERO, Vec3::new(half, 0.05, half)),
    }
}

/// Smooth value noise on a wrapping grid of `cells` x `cells` random values.
fn value_noise(x: f32, y: f32, cells: u32, salt: u32) -> f32 {
    let (fx, fy) = (x * cells as f32, y * cells as f32);
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let corner = |cx: u32, cy: u32| {
        unit((cx % cells).wrapping_mul(73_856_093) ^ (cy % cells).wrapping_mul(19_349_663) ^ salt)
    };
    let top = corner(x0, y0) + (corner(x0 + 1, y0) - corner(x0, y0)) * smooth(tx);
    let bottom = corner(x0, y0 + 1) + (corner(x0 + 1, y0 + 1) - corner(x0, y0 + 1)) * smooth(tx);
    top + (bottom - top) * smooth(ty)
}

/// A grass texture that tiles: soft patches of light and dark with fine blade speckle.
fn grass_png() -> Vec<u8> {
    const SIZE: u32 = 512;
    let mut image = image::RgbaImage::new(SIZE, SIZE);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let (u, v) = (x as f32 / SIZE as f32, y as f32 / SIZE as f32);
        let patches = value_noise(u, v, 4, 1) * 0.5 + value_noise(u, v, 8, 2) * 0.3;
        let blades = value_noise(u, v, 128, 3) * 0.25 + value_noise(u, v, 64, 4) * 0.15;
        let n = (patches + blades).clamp(0.0, 1.0);
        let r = (24.0 + 46.0 * n) as u8;
        let g = (66.0 + 84.0 * n) as u8;
        let b = (22.0 + 28.0 * n) as u8;
        *pixel = image::Rgba([r, g, b, 255]);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("PNG encoding into memory");
    out.into_inner()
}

fn emissive(base: Vec3, glow: Vec3) -> Material {
    Material::solid(base)
        .metallic(0.2)
        .roughness(0.12)
        .emissive(glow)
}

fn bounds_of(r: &Renderer, mesh: usize, model: Mat4) -> Aabb {
    r.get_mesh_bounds(mesh).map_or(
        Aabb {
            min: Vec3::splat(-1.0e9),
            max: Vec3::splat(1.0e9),
        },
        |b| b.transform(model),
    )
}

fn object(r: &Renderer, model: Mat4, mesh: usize, material: usize) -> RenderObject {
    RenderObject {
        model_matrix: model,
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: bounds_of(r, mesh, model),
    }
}

fn scaled(center: Vec3, scale: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center)
}

impl Scene {
    pub fn new(r: &mut Renderer, fox: &FoxAssets) -> Result<Self, String> {
        let sphere = r.upload_mesh(&outward(primitive::create_sphere(1.0, 24)));
        let ground = r.upload_mesh(&ground_mesh(ARENA_RADIUS * 3.0, 4.0));
        let grass_texture = r
            .upload_texture(&grass_png(), "grass")
            .map_err(|e| e.to_string())?;
        let grass = r.add_material(Material::textured(Vec3::ONE, grass_texture).roughness(0.95));
        let trunk = r.add_material(Material::solid(Vec3::new(0.30, 0.19, 0.11)).roughness(0.9));
        let foliage = [
            Vec3::new(0.08, 0.30, 0.14),
            Vec3::new(0.12, 0.36, 0.12),
            Vec3::new(0.18, 0.40, 0.10),
            Vec3::new(0.72, 0.34, 0.08),
        ]
        .map(|c| r.add_material(Material::solid(c).roughness(0.8)));
        let rock = r.add_material(Material::solid(Vec3::new(0.34, 0.35, 0.38)).roughness(0.92));
        let gems = [
            emissive(Vec3::new(0.2, 0.8, 1.0), Vec3::new(0.3, 1.5, 2.4)),
            emissive(Vec3::new(1.0, 0.3, 0.9), Vec3::new(2.4, 0.4, 1.9)),
            emissive(Vec3::new(1.0, 0.8, 0.2), Vec3::new(2.6, 1.8, 0.3)),
        ]
        .map(|m| r.add_material(m));
        let wisp = r.add_material(emissive(
            Vec3::new(0.06, 0.02, 0.12),
            Vec3::new(0.9, 0.15, 1.6),
        ));

        let texture = r
            .upload_texture(&fox.texture_png, "fox")
            .map_err(|e| e.to_string())?;
        let fox_material = r.add_material(Material::textured(Vec3::ONE, texture).roughness(0.75));
        let bind = fox
            .model
            .geometry
            .positions
            .iter()
            .fold(Aabb::empty(), |b, p| b.expand(Vec3::from(*p)));
        let binding = SkinnedMeshBinding::new(
            r,
            &fox.model.geometry,
            1.5 * bind.half_extents().max_element(),
        );
        Ok(Self {
            sphere,
            ground,
            grass,
            trunk,
            foliage,
            rock,
            gems,
            wisp,
            fox_material,
            fox: binding,
        })
    }

    /// Poses the fox: skins the mesh with the animation output.
    pub fn pose_fox(&mut self, r: &Renderer, fox: &FoxAssets, pose: &redox_anim::LocalPose) {
        let skin = Pose::from_local(&fox.model.skeleton, pose).skin_matrices(&fox.model.skeleton);
        if !self.fox.update_gpu(r, &skin) && !self.fox.update(r, &skin) {
            log::warn!("skinned mesh update rejected");
        }
    }

    /// A pine-like tree: a trunk and three crown blobs, all in proportion to `size`
    /// (trunk radius is `TRUNK * size`, the crown is `CROWN * size` wide).
    fn push_tree(
        &self,
        r: &Renderer,
        frame: &mut Frame,
        id: u64,
        (x, z): (f32, f32),
        size: f32,
        variant: u32,
    ) {
        let leaves = self.foliage[(hash(variant ^ 0x55) % 4) as usize];
        let parts = [
            (self.trunk, 1.0, Vec3::new(TRUNK, 1.65, TRUNK)),
            (leaves, 2.6, Vec3::new(CROWN, 1.0, CROWN)),
            (leaves, 3.4, Vec3::new(CROWN * 0.76, 0.85, CROWN * 0.76)),
            (leaves, 4.1, Vec3::new(CROWN * 0.48, 0.7, CROWN * 0.48)),
        ];
        for (k, (material, height, shape)) in parts.into_iter().enumerate() {
            let center = Vec3::new(x, height * size, z);
            frame.objects.push(object(
                r,
                scaled(center, shape * size),
                self.sphere,
                material,
            ));
            frame.ids.push(id + k as u64);
        }
    }

    /// Objects and lights of the current state of the game.
    pub fn frame(&self, r: &Renderer, world: &World, time: f32, fox: &FoxAssets) -> Frame {
        let mut frame = Frame {
            objects: Vec::new(),
            ids: Vec::new(),
            lights: lighting(),
        };
        frame
            .objects
            .push(object(r, Mat4::IDENTITY, self.ground, self.grass));
        frame.ids.push(1);

        // The fox.
        let (x, z) = world.pos;
        let model = Mat4::from_scale_rotation_translation(
            Vec3::splat(MODEL_SCALE),
            Quat::from_rotation_y(world.yaw),
            Vec3::new(x, 0.0, z),
        );
        frame.objects.push(RenderObject {
            model_matrix: model,
            color: if world.invulnerable() && (time * 14.0).sin() > 0.0 {
                [1.0, 0.45, 0.45, 1.0]
            } else {
                [1.0; 4]
            },
            mesh_index: self.fox.mesh_slot,
            material_index: self.fox_material,
            bounds: self.fox.bounds.transform(model),
        });
        frame.ids.push(2);
        let _ = fox;

        // Trees and rocks the fox collides with.
        for (i, o) in world.obstacles.iter().enumerate() {
            let id = 1000 + 8 * i as u64;
            if o.tree {
                self.push_tree(r, &mut frame, id, (o.x, o.z), o.radius / TRUNK, i as u32);
            } else {
                let scale = Vec3::new(o.radius * 1.15, o.radius * 0.7, o.radius);
                frame.objects.push(object(
                    r,
                    scaled(Vec3::new(o.x, o.radius * 0.35, o.z), scale),
                    self.sphere,
                    self.rock,
                ));
                frame.ids.push(id);
            }
        }

        // The forest around the glade.
        for i in 0..RIM_TREES {
            let angle = std::f32::consts::TAU * (i as f32 + unit(i as u32 * 7)) / RIM_TREES as f32;
            let distance = ARENA_RADIUS + 4.5 + 12.0 * unit(i as u32 * 13 + 1);
            let size = 2.0 + 1.2 * unit(i as u32 * 31 + 2);
            self.push_tree(
                r,
                &mut frame,
                5000 + 8 * i as u64,
                (angle.cos() * distance, angle.sin() * distance),
                size,
                1000 + i as u32,
            );
        }

        // Gems: bobbing, spinning, glowing; the nearest ones also light their surroundings.
        let mut lit: Vec<(f32, Vec3, Vec3)> = Vec::new();
        for (i, gem) in world.gems.iter().enumerate() {
            if gem.taken {
                continue;
            }
            let bob = 0.75 + 0.12 * (time * 2.2 + i as f32).sin();
            let center = Vec3::new(gem.x, bob, gem.z);
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(0.22, 0.38, 0.22),
                Quat::from_rotation_y(time * 1.8 + i as f32),
                center,
            );
            let material = self.gems[i % 3];
            frame.objects.push(object(r, model, self.sphere, material));
            frame.ids.push(3000 + i as u64);
            let glow = [
                Vec3::new(0.3, 0.8, 1.0),
                Vec3::new(1.0, 0.3, 0.9),
                Vec3::new(1.0, 0.75, 0.25),
            ][i % 3];
            let d = (gem.x - x).hypot(gem.z - z);
            lit.push((d, center + Vec3::Y * 0.4, glow));
        }
        lit.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (distance, position, color) in lit.into_iter().take(GEM_LIGHTS) {
            let fade = ((GEM_LIGHT_FADE_END - distance)
                / (GEM_LIGHT_FADE_END - GEM_LIGHT_FADE_START))
                .clamp(0.0, 1.0);
            if fade > 0.0 {
                frame
                    .lights
                    .points
                    .push(PointLight::new(position, color, 38.0 * fade, 6.0));
            }
        }

        // Wisps.
        for (i, wisp) in world.wisps.iter().enumerate() {
            let (wx, wz) = wisp.position(world.elapsed);
            let float = 0.9 + 0.25 * (time * 3.0 + i as f32 * 1.7).sin();
            let pulse = 0.45 + 0.08 * (time * 7.0 + i as f32).sin();
            let center = Vec3::new(wx, float, wz);
            frame.objects.push(object(
                r,
                scaled(center, Vec3::splat(pulse)),
                self.sphere,
                self.wisp,
            ));
            frame.ids.push(4000 + i as u64);
            frame
                .lights
                .points
                .push(PointLight::new(center, Vec3::new(0.7, 0.2, 1.0), 22.0, 4.5));
        }
        frame
    }
}

/// Late-afternoon sun and a cool sky ambient.
pub fn lighting() -> LightScene {
    let mut sun =
        DirectionalLight::new(Vec3::new(-0.5, 0.85, 0.35), Vec3::new(1.0, 0.82, 0.62), 2.6);
    sun.soft_shadows = false;
    sun.vsm = true;
    LightScene {
        directional: sun,
        ambient: Vec3::new(0.10, 0.13, 0.20),
        points: vec![],
        spots: vec![],
    }
}

/// Chase camera behind the fox, looking a little ahead of it.
pub fn camera_target(world: &World, orbit: f32) -> (Vec3, Vec3) {
    let focus = Vec3::new(world.pos.0, 0.8, world.pos.1);
    let yaw = world.camera_yaw + orbit;
    let behind = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    (focus - behind * 5.8 + Vec3::Y * 2.6, focus + behind * 1.0)
}

/// Moves the camera out of trunks and rocks and keeps it inside the ring of rim trees.
pub fn keep_clear(world: &World, mut eye: Vec3) -> Vec3 {
    for o in &world.obstacles {
        let (height, clear) = if o.tree {
            (6.5, o.radius / TRUNK * CROWN + 0.7)
        } else {
            (o.radius * 1.2, o.radius + 1.0)
        };
        let (dx, dz) = (eye.x - o.x, eye.z - o.z);
        let d = dx.hypot(dz);
        if d < clear && eye.y < height {
            let (nx, nz) = if d > 1e-3 {
                (dx / d, dz / d)
            } else {
                (1.0, 0.0)
            };
            eye.x = o.x + nx * clear;
            eye.z = o.z + nz * clear;
        }
    }
    let limit = ARENA_RADIUS + 0.5;
    let d = eye.x.hypot(eye.z);
    if d > limit {
        eye.x *= limit / d;
        eye.z *= limit / d;
    }
    eye
}

/// Rotation that looks from `eye` at `target`.
pub fn look_at(eye: Vec3, target: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::NEG_Z, (target - eye).normalize_or_zero())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ground_faces_up_and_tiles_its_uvs() {
        let mesh = ground_mesh(40.0, 4.0);
        assert_eq!(mesh.indices.len(), 6);
        let p = |i: usize| Vec3::from(mesh.vertices[mesh.indices[i] as usize].position);
        let normal = (p(1) - p(0)).cross(p(2) - p(0));
        assert!(normal.y > 0.0, "{normal:?}");
        assert_eq!(mesh.vertices[2].uv, [10.0, 10.0]);
    }

    #[test]
    fn the_grass_texture_is_a_valid_png_with_variation() {
        let png = grass_png();
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!((image.width(), image.height()), (512, 512));
        let greens: Vec<u8> = image.pixels().map(|p| p[1]).collect();
        let (min, max) = (
            greens.iter().copied().min().unwrap(),
            greens.iter().copied().max().unwrap(),
        );
        assert!(max - min > 40, "flat texture: {min}..{max}");
    }

    #[test]
    fn spheres_face_outward_after_the_fix() {
        let mesh = outward(primitive::create_sphere(1.0, 16));
        let (mut outward_faces, mut real_faces) = (0, 0);
        for t in mesh.indices.chunks_exact(3) {
            let p = |i: usize| Vec3::from(mesh.vertices[t[i] as usize].position);
            let normal = (p(1) - p(0)).cross(p(2) - p(0));
            if normal.length() < 1e-6 {
                continue; // the triangles collapsed at the poles
            }
            real_faces += 1;
            let centre = (p(0) + p(1) + p(2)) / 3.0;
            if normal.dot(centre) > 0.0 {
                outward_faces += 1;
            }
        }
        assert!(real_faces > 200);
        assert_eq!(
            outward_faces, real_faces,
            "every real triangle faces outward"
        );
    }

    #[test]
    fn the_camera_is_pushed_out_of_trees_and_the_rim() {
        let mut world = World::new(1);
        world.obstacles = vec![crate::world::Obstacle {
            x: 5.0,
            z: 5.0,
            radius: 0.5,
            tree: true,
        }];
        let inside = keep_clear(&world, Vec3::new(5.2, 2.0, 5.0));
        assert!((inside.x - 5.0).hypot(inside.z - 5.0) >= 0.5 / TRUNK * CROWN - 1e-3);
        let above = keep_clear(&world, Vec3::new(5.2, 9.0, 5.0));
        assert_eq!(above, Vec3::new(5.2, 9.0, 5.0), "over the crown is fine");
        let far = keep_clear(&world, Vec3::new(80.0, 2.0, 0.0));
        assert!(far.x.hypot(far.z) <= ARENA_RADIUS + 0.6);
    }

    #[test]
    fn the_hash_noise_is_stable_and_in_range() {
        for i in 0..1000 {
            let v = unit(i);
            assert!((0.0..=1.0).contains(&v));
        }
        assert_eq!(unit(5), unit(5));
        assert_ne!(unit(5), unit(6));
    }

    #[test]
    fn the_camera_sits_behind_and_above_the_fox() {
        let world = World::new(1);
        let (eye, target) = camera_target(&world, 0.0);
        assert!(eye.y > target.y);
        let (fx, fz) = world.camera_forward();
        let toward = Vec3::new(fx, 0.0, fz);
        assert!(
            (target - eye).dot(toward) > 5.0,
            "looks forward along the camera yaw"
        );
    }
}
