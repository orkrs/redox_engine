//! 3D Procedural map geometry, materials, and mouse raycasting.

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::mesh::{Mesh, Vertex};
use redox_render::{RenderObject, Renderer};
use std::collections::HashMap;

use super::sim::Simulation;
use super::types::*;

/// Creates a procedural cylinder/disc with top & bottom caps and side faces.
fn create_cylinder(radius: f32, height: f32, segments: u32) -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let half_h = height * 0.5;

    // Top cap center
    let top_center_idx = vertices.len() as u32;
    vertices.push(Vertex {
        position: [0.0, half_h, 0.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.5, 0.5],
        tangent: [1.0, 0.0, 0.0],
    });

    for i in 0..=segments {
        let angle = i as f32 * std::f32::consts::TAU / segments as f32;
        let (sin, cos) = angle.sin_cos();
        vertices.push(Vertex {
            position: [cos * radius, half_h, sin * radius],
            normal: [0.0, 1.0, 0.0],
            uv: [0.5 + cos * 0.5, 0.5 + sin * 0.5],
            tangent: [1.0, 0.0, 0.0],
        });
    }

    for i in 0..segments {
        indices.extend_from_slice(&[
            top_center_idx,
            top_center_idx + 2 + i,
            top_center_idx + 1 + i,
        ]);
    }

    // Bottom cap center
    let bot_center_idx = vertices.len() as u32;
    vertices.push(Vertex {
        position: [0.0, -half_h, 0.0],
        normal: [0.0, -1.0, 0.0],
        uv: [0.5, 0.5],
        tangent: [1.0, 0.0, 0.0],
    });

    for i in 0..=segments {
        let angle = i as f32 * std::f32::consts::TAU / segments as f32;
        let (sin, cos) = angle.sin_cos();
        vertices.push(Vertex {
            position: [cos * radius, -half_h, sin * radius],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5 + cos * 0.5, 0.5 + sin * 0.5],
            tangent: [1.0, 0.0, 0.0],
        });
    }

    for i in 0..segments {
        indices.extend_from_slice(&[
            bot_center_idx,
            bot_center_idx + 1 + i,
            bot_center_idx + 2 + i,
        ]);
    }

    // Sides
    let side_start = vertices.len() as u32;
    for i in 0..=segments {
        let angle = i as f32 * std::f32::consts::TAU / segments as f32;
        let (sin, cos) = angle.sin_cos();
        let u = i as f32 / segments as f32;
        vertices.push(Vertex {
            position: [cos * radius, half_h, sin * radius],
            normal: [cos, 0.0, sin],
            uv: [u, 0.0],
            tangent: [-sin, 0.0, cos],
        });
        vertices.push(Vertex {
            position: [cos * radius, -half_h, sin * radius],
            normal: [cos, 0.0, sin],
            uv: [u, 1.0],
            tangent: [-sin, 0.0, cos],
        });
    }

    for i in 0..segments {
        let top_left = side_start + i * 2;
        let bot_left = side_start + i * 2 + 1;
        let top_right = side_start + (i + 1) * 2;
        let bot_right = side_start + (i + 1) * 2 + 1;
        indices.extend_from_slice(&[top_left, top_right, bot_left]);
        indices.extend_from_slice(&[top_right, bot_right, bot_left]);
    }

    Mesh::new(vertices, indices)
}

pub struct MapAssets {
    pub mesh_slab: usize,
    pub mesh_fort: usize,
    pub mesh_road: usize,
    pub mesh_army: usize,
    pub mesh_selection: usize,
    pub mat_board: usize,
    pub mat_neutral: usize,
    pub mat_fort: usize,
    pub mat_road: usize,
    pub mat_selection: usize,
    pub country_materials: HashMap<CountryTag, usize>,
}

impl MapAssets {
    pub fn new(r: &mut Renderer) -> Self {
        // 1. Procedural meshes
        // Slab for province territory (flat beveled cylinder/disc)
        let slab_mesh = create_cylinder(0.95, 0.12, 16);
        let mesh_slab = r.upload_mesh(&slab_mesh);

        // Fortification / Kremlin tower (cube with tower cap)
        let fort_mesh = create_cube();
        let mesh_fort = r.upload_mesh(&fort_mesh);

        // Road connector (unit elongated cube)
        let road_mesh = create_cube();
        let mesh_road = r.upload_mesh(&road_mesh);

        // Army token (sphere on cylinder pedestal)
        let army_mesh = create_sphere(0.35, 12);
        let mesh_army = r.upload_mesh(&army_mesh);

        // Selection highlight ring
        let sel_mesh = create_cylinder(1.08, 0.04, 20);
        let mesh_selection = r.upload_mesh(&sel_mesh);

        // 2. Materials
        let mat_board = r.add_material(Material::solid(Vec3::new(0.12, 0.14, 0.10)).roughness(0.9));
        let mat_neutral =
            r.add_material(Material::solid(Vec3::new(0.35, 0.38, 0.32)).roughness(0.8));
        let mat_fort = r.add_material(Material::solid(Vec3::new(0.85, 0.85, 0.80)).roughness(0.6)); // Белокаменный известняк
        let mat_road = r.add_material(Material::solid(Vec3::new(0.28, 0.24, 0.18)).roughness(0.95)); // Грунтовый тракт
        let mat_selection = r.add_material(
            Material::solid(Vec3::new(1.0, 0.85, 0.1)).emissive(Vec3::new(0.6, 0.5, 0.05)),
        );

        let mut country_materials = HashMap::new();
        let all_tags = [
            CountryTag::Moscow,
            CountryTag::Novgorod,
            CountryTag::Tver,
            CountryTag::Ryazan,
            CountryTag::Suzdal,
            CountryTag::Lithuania,
            CountryTag::GoldenHorde,
            CountryTag::Pskov,
        ];

        for tag in all_tags {
            let col = tag.map_color();
            let mat_id = r.add_material(
                Material::solid(Vec3::new(col[0], col[1], col[2]))
                    .roughness(0.65)
                    .metallic(0.1),
            );
            country_materials.insert(tag, mat_id);
        }

        Self {
            mesh_slab,
            mesh_fort,
            mesh_road,
            mesh_army,
            mesh_selection,
            mat_board,
            mat_neutral,
            mat_fort,
            mat_road,
            mat_selection,
            country_materials,
        }
    }

    /// Build render objects for the current frame based on simulation state.
    pub fn build_render_objects(
        &self,
        sim: &Simulation,
        map_mode: MapMode,
        selected_province: Option<ProvinceId>,
        hovered_province: Option<ProvinceId>,
        selected_army: Option<u32>,
    ) -> Vec<RenderObject> {
        let mut objects = Vec::with_capacity(128);

        // 1. Board background table
        let board_model = Mat4::from_scale_rotation_translation(
            Vec3::new(26.0, 0.1, 24.0),
            Quat::IDENTITY,
            Vec3::new(0.0, -0.15, 0.0),
        );
        objects.push(RenderObject {
            id: 0,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: board_model,
            color: [0.15, 0.18, 0.13, 1.0],
            mesh_index: self.mesh_fort,
            material_index: self.mat_board,
            bounds: Aabb {
                min: Vec3::splat(-30.0),
                max: Vec3::splat(30.0),
            },
        });

        // 2. Roads between neighbor provinces
        let mut drawn_pairs: Vec<(ProvinceId, ProvinceId)> = Vec::new();
        for prov in sim.provinces.values() {
            for &n_id in &prov.neighbors {
                if let Some(neighbor) = sim.provinces.get(&n_id) {
                    let pair = if prov.id.0 < n_id.0 {
                        (prov.id, n_id)
                    } else {
                        (n_id, prov.id)
                    };
                    if !drawn_pairs.contains(&pair) {
                        drawn_pairs.push(pair);
                        let p1 = prov.position;
                        let p2 = neighbor.position;
                        let mid = (p1 + p2) * 0.5 + Vec3::new(0.0, 0.02, 0.0);
                        let diff = p2 - p1;
                        let len = diff.length();
                        let dir = diff / len;
                        let angle = dir.x.atan2(dir.z);
                        let rot = Quat::from_rotation_y(angle);
                        let model = Mat4::from_scale_rotation_translation(
                            Vec3::new(0.08, 0.02, len),
                            rot,
                            mid,
                        );
                        objects.push(RenderObject {
                            id: 0,
                            shadow_class: redox_render::shadow::ShadowClass::Auto,
                            deforms: false,
                            pose_version: 0,
                            model_matrix: model,
                            color: [0.35, 0.30, 0.22, 1.0],
                            mesh_index: self.mesh_road,
                            material_index: self.mat_road,
                            bounds: Aabb {
                                min: Vec3::splat(-15.0),
                                max: Vec3::splat(15.0),
                            },
                        });
                    }
                }
            }
        }

        // 3. Province Territory Slabs
        for prov in sim.provinces.values() {
            let is_selected = selected_province == Some(prov.id);
            let is_hovered = hovered_province == Some(prov.id);

            let lift = if is_selected {
                0.12
            } else if is_hovered {
                0.06
            } else {
                0.0
            };
            let pos = prov.position + Vec3::new(0.0, lift, 0.0);

            // Determine color & material by MapMode
            let (mat_idx, color_tint) = match map_mode {
                MapMode::Political => {
                    let mat = self
                        .country_materials
                        .get(&prov.controller)
                        .copied()
                        .unwrap_or(self.mat_neutral);
                    let tint = if is_selected {
                        [1.3, 1.3, 1.3, 1.0]
                    } else if is_hovered {
                        [1.15, 1.15, 1.15, 1.0]
                    } else {
                        [1.0, 1.0, 1.0, 1.0]
                    };
                    (mat, tint)
                }
                MapMode::Demographics => {
                    let total = prov.total_population() as f32;
                    let t = (total / 60000.0).clamp(0.0, 1.0);
                    let r_col = 0.2 + t * 0.8;
                    let g_col = 0.7 - t * 0.3;
                    let b_col = 0.9 * (1.0 - t);
                    (self.mat_neutral, [r_col, g_col, b_col, 1.0])
                }
                MapMode::TradeGoods => {
                    let col = prov.trade_good.color();
                    (self.mat_neutral, col)
                }
                MapMode::Control => {
                    let c = prov.control;
                    (self.mat_neutral, [1.0 - c, c, 0.15, 1.0])
                }
            };

            let slab_model = Mat4::from_scale_rotation_translation(
                Vec3::new(1.0, 1.0, 1.0),
                Quat::IDENTITY,
                pos,
            );

            objects.push(RenderObject {
                id: prov.id.0 as u32,
                shadow_class: redox_render::shadow::ShadowClass::Auto,
                deforms: false,
                pose_version: 0,
                model_matrix: slab_model,
                color: color_tint,
                mesh_index: self.mesh_slab,
                material_index: mat_idx,
                bounds: Aabb {
                    min: Vec3::splat(-10.0),
                    max: Vec3::splat(10.0),
                },
            });

            // Selection ring
            if is_selected {
                let sel_model = Mat4::from_scale_rotation_translation(
                    Vec3::new(1.0, 1.0, 1.0),
                    Quat::IDENTITY,
                    pos + Vec3::new(0.0, 0.05, 0.0),
                );
                objects.push(RenderObject {
                    id: 0,
                    shadow_class: redox_render::shadow::ShadowClass::Auto,
                    deforms: false,
                    pose_version: 0,
                    model_matrix: sel_model,
                    color: [1.0, 0.9, 0.2, 1.0],
                    mesh_index: self.mesh_selection,
                    material_index: self.mat_selection,
                    bounds: Aabb {
                        min: Vec3::splat(-10.0),
                        max: Vec3::splat(10.0),
                    },
                });
            }

            // Kremlin / Fort model on capital or fortified cities
            if prov.fort_level > 0 || prov.is_capital {
                let h = if prov.is_capital { 0.45 } else { 0.30 };
                let fort_model = Mat4::from_scale_rotation_translation(
                    Vec3::new(0.28, h, 0.28),
                    Quat::IDENTITY,
                    pos + Vec3::new(0.0, h * 0.5 + 0.06, 0.0),
                );
                objects.push(RenderObject {
                    id: 0,
                    shadow_class: redox_render::shadow::ShadowClass::Auto,
                    deforms: false,
                    pose_version: 0,
                    model_matrix: fort_model,
                    color: if prov.is_capital {
                        [1.0, 0.9, 0.7, 1.0]
                    } else {
                        [0.8, 0.8, 0.8, 1.0]
                    },
                    mesh_index: self.mesh_fort,
                    material_index: self.mat_fort,
                    bounds: Aabb {
                        min: Vec3::splat(-10.0),
                        max: Vec3::splat(10.0),
                    },
                });
            }
        }

        // 4. Armies on Map
        for army in &sim.armies {
            let base_pos = if let Some(prov) = sim.provinces.get(&army.location) {
                if !army.movement_path.is_empty() {
                    let next_id = army.movement_path[0];
                    if let Some(next_prov) = sim.provinces.get(&next_id) {
                        prov.position
                            .lerp(next_prov.position, army.movement_progress)
                    } else {
                        prov.position
                    }
                } else {
                    prov.position
                }
            } else {
                Vec3::ZERO
            };

            let is_sel = selected_army == Some(army.id);
            let army_pos = base_pos + Vec3::new(0.0, 0.35, 0.0);
            let army_scale = if is_sel {
                Vec3::splat(1.3)
            } else {
                Vec3::splat(1.0)
            };

            let army_model =
                Mat4::from_scale_rotation_translation(army_scale, Quat::IDENTITY, army_pos);

            let army_mat = self
                .country_materials
                .get(&army.owner)
                .copied()
                .unwrap_or(self.mat_neutral);

            objects.push(RenderObject {
                id: army.id + 1000,
                shadow_class: redox_render::shadow::ShadowClass::Auto,
                deforms: false,
                pose_version: 0,
                model_matrix: army_model,
                color: if army.in_battle {
                    [1.5, 0.3, 0.3, 1.0]
                } else if is_sel {
                    [1.2, 1.2, 0.4, 1.0]
                } else {
                    [1.0, 1.0, 1.0, 1.0]
                },
                mesh_index: self.mesh_army,
                material_index: army_mat,
                bounds: Aabb {
                    min: Vec3::splat(-10.0),
                    max: Vec3::splat(10.0),
                },
            });
        }

        objects
    }

    /// Cast a ray from screen coordinates to find the hovered or clicked province.
    pub fn pick_province(
        mouse_x: f32,
        mouse_y: f32,
        win_w: f32,
        win_h: f32,
        inv_view_proj: Mat4,
        provinces: &HashMap<ProvinceId, Province>,
    ) -> Option<ProvinceId> {
        let ndc_x = (2.0 * mouse_x / win_w) - 1.0;
        let ndc_y = 1.0 - (2.0 * mouse_y / win_h);

        let p_near = inv_view_proj.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let p_far = inv_view_proj.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));

        let dir = (p_far - p_near).normalize();
        if dir.y.abs() < 1e-6 {
            return None;
        }

        let t = -p_near.y / dir.y;
        if t <= 0.0 {
            return None;
        }

        let hit = p_near + dir * t;

        provinces
            .values()
            .filter(|p| (p.position.x - hit.x).hypot(p.position.z - hit.z) <= 0.95)
            .min_by(|a, b| {
                let dist_a = (a.position.x - hit.x).hypot(a.position.z - hit.z);
                let dist_b = (b.position.x - hit.x).hypot(b.position.z - hit.z);
                dist_a
                    .partial_cmp(&dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id)
    }
}
