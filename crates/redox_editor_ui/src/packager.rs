//! Project packager / bake panel — OFPA inventory, CPU meshlet/SDF bake, package export.

use std::path::{Path, PathBuf};

use redox_asset::load_scene_from_path;
use redox_math::Vec3;
use redox_render::mesh::Mesh;
use redox_render::sdf::{SdfBrick, SdfBrickAtlas, bake_mesh};
use redox_render::virtual_geometry::{LodChainConfig, build_vg_asset, cache as vg_cache};
use redox_scene::{
    CELLS_DIR, PcgKind, PcgParams, generate_to, load_cell_actors, load_manifest, parse_cell_dir,
};

/// Result of baking a world for shipping.
#[derive(Clone, Debug)]
pub struct BakeReport {
    /// Root directory of the baked world.
    pub world_root: PathBuf,
    /// Number of cells.
    pub cell_count: usize,
    /// Number of actor files.
    pub actor_files: usize,
    /// Where the report was written.
    pub report_path: PathBuf,
    /// Lines of the report.
    pub lines: Vec<String>,
}

/// Outcome of a CPU meshlet bake (GPU registration happens in `redox_editor::RenderHost`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshletBakeOutcome {
    /// Disk cache key.
    pub cache_key: u64,
    /// `.cache/vg/{cache_key:016x}.rmesh`
    pub cache_path: PathBuf,
    /// glTF or mesh source that was clustered.
    pub source: PathBuf,
    /// Number of meshlets in the asset.
    pub meshlet_count: usize,
    /// `gpu_register: ok <id>` or a failure reason for the report file.
    pub gpu_register_line: String,
}

/// Status of the CPU bakes.
#[derive(Clone, Debug, Default)]
pub struct BakeStatus {
    /// Result of the meshlet bake.
    pub meshlets: String,
    /// Result of the SDF bake.
    pub sdf: String,
}

/// Panel that inventories a world and exports a shipping package.
#[derive(Default)]
pub struct PackagerPanel {
    /// Root directory of the world.
    pub world_root: String,
    /// Optional editor project root for `redox_editor_build::BuildService::cook`.
    pub project_root: String,
    /// Build profile name for project cook (default `release`).
    pub cook_profile: String,
    /// Name of the exported package.
    pub package_name: String,
    /// Report of the last bake.
    pub last_report: Option<BakeReport>,
    /// Status of the bakes.
    pub bake_status: BakeStatus,
    /// Message shown at the bottom of the panel.
    pub status: String,
}

impl PackagerPanel {
    /// Creates the panel for a world; the package is named after its directory.
    pub fn new(world_root: impl Into<String>) -> Self {
        let world_root = world_root.into();
        let package_name = Path::new(&world_root)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("package")
            .to_string();
        Self {
            world_root,
            project_root: String::new(),
            cook_profile: "release".into(),
            package_name,
            ..Default::default()
        }
    }

    /// Runs project cook through [`BuildService`] (no UI logic beyond I/O paths).
    pub fn cook_project(&self) -> anyhow::Result<String> {
        if self.project_root.is_empty() {
            anyhow::bail!("set project root before cooking");
        }
        let project = redox_editor_core::project::ProjectContext::open(&self.project_root)?;
        let service = redox_editor_build::BuildService;
        let profile = service
            .resolve_active(project.manifest(), Some(self.cook_profile.as_str()))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let out_dir = project.root().join(".redox");
        std::fs::create_dir_all(&out_dir)?;
        let pak = out_dir.join(format!("{}.pak", profile.name));
        let report = out_dir.join("cook_report.json");
        let cooked = service.cook(&project, &profile, &pak, &report)?;
        Ok(format!(
            "cooked {} assets → {} (report {})",
            cooked.assets.len(),
            pak.display(),
            report.display()
        ))
    }

    /// Draws the panel.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading("Project Packager");
        ui.label("OFPA inventory, CPU meshlet/SDF bake, shipping package export");
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("World root");
            ui.text_edit_singleline(&mut self.world_root);
        });
        ui.horizontal(|ui| {
            ui.label("Project root");
            ui.text_edit_singleline(&mut self.project_root);
        });
        ui.horizontal(|ui| {
            ui.label("Cook profile");
            ui.text_edit_singleline(&mut self.cook_profile);
        });
        ui.horizontal(|ui| {
            ui.label("Package name");
            ui.text_edit_singleline(&mut self.package_name);
        });

        ui.horizontal(|ui| {
            if ui.button("Scan OFPA").clicked() {
                match scan_ofpa(Path::new(&self.world_root)) {
                    Ok(report) => {
                        self.status = format!(
                            "Scanned {} cells, {} actor files",
                            report.cell_count, report.actor_files
                        );
                        self.last_report = Some(report);
                    }
                    Err(e) => self.status = format!("Scan failed: {e}"),
                }
            }
            if ui.button("Write Bake Report").clicked()
                && let Err(e) = self.write_bake_report()
            {
                self.status = format!("Bake report failed: {e}");
            }
            if ui.button("Generate PCG Arena").clicked() {
                match bake_pcg_arena(Path::new(&self.world_root), false) {
                    Ok(msg) => self.status = msg,
                    Err(e) => self.status = format!("PCG failed: {e}"),
                }
            }
            if ui.button("Bake PCG → World").clicked() {
                match bake_pcg_arena(Path::new(&self.world_root), true) {
                    Ok(msg) => self.status = msg,
                    Err(e) => self.status = format!("PCG bake failed: {e}"),
                }
            }
        });

        ui.horizontal(|ui| {
            if ui.button("Bake Meshlets").clicked() {
                self.status = "queue_bake_meshlets".into();
            }
            if ui.button("Bake SDF").clicked() {
                match self.bake_sdf() {
                    Ok(msg) => self.status = msg,
                    Err(e) => self.status = format!("SDF bake failed: {e}"),
                }
            }
            if ui.button("Export Package").clicked() {
                match self.export_package() {
                    Ok(msg) => self.status = msg,
                    Err(e) => self.status = format!("Export failed: {e}"),
                }
            }
            if ui.button("Cook project").clicked() {
                match self.cook_project() {
                    Ok(msg) => self.status = msg,
                    Err(e) => self.status = format!("Cook failed: {e}"),
                }
            }
        });

        if !self.status.is_empty() {
            ui.colored_label(egui::Color32::from_rgb(180, 220, 255), &self.status);
        }

        ui.separator();
        ui.label("Checklist");
        ui.monospace("1. Scan OFPA cells (redox_scene::load_manifest / cells)");
        ui.monospace("2. Write bake report (.cache/bake_report.txt)");
        ui.monospace("3. PCG preview (.cache) or bake into world root");
        ui.monospace("4. Bake Meshlets — build_vg_asset + vg_cache::store_cached");
        ui.monospace("5. Bake SDF — bake_mesh (+ GPU atlas needs Renderer)");
        ui.monospace("6. Export Package → .cache/packages/<name>/");

        if !self.bake_status.meshlets.is_empty() {
            ui.label(format!("Meshlets: {}", self.bake_status.meshlets));
        }
        if !self.bake_status.sdf.is_empty() {
            ui.label(format!("SDF: {}", self.bake_status.sdf));
        }

        if let Some(report) = &self.last_report {
            ui.separator();
            ui.label(format!("Last: {}", report.report_path.display()));
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for line in &report.lines {
                        ui.monospace(line);
                    }
                });
        }
    }

    /// Writes the bake report and returns a status message.
    pub fn write_bake_report(&mut self) -> anyhow::Result<String> {
        let report = write_bake_report(Path::new(&self.world_root), &self.bake_status)?;
        let msg = format!("Wrote {}", report.report_path.display());
        self.status = msg.clone();
        self.last_report.replace(report);
        Ok(msg)
    }

    /// CPU meshlet bake via [`build_vg_asset`] + disk cache. Pass mesh glTF paths; empty → error.
    pub fn bake_meshlets(&mut self) -> anyhow::Result<String> {
        self.bake_meshlets_from_paths(&[])
    }

    /// Bakes the first resolvable mesh in `sources` (OFPA has no mesh paths — use asset browser glTF).
    pub fn bake_meshlets_from_paths(
        &mut self,
        sources: &[PathBuf],
    ) -> anyhow::Result<MeshletBakeOutcome> {
        if sources.is_empty() {
            anyhow::bail!("no mesh selected — pick a glTF in the asset browser (OFPA actors have no mesh paths)");
        }
        let source = sources
            .iter()
            .find(|p| p.is_file())
            .ok_or_else(|| anyhow::anyhow!("no readable mesh file in selection"))?;
        let mesh = load_mesh_for_vg(source)?;
        self.bake_meshlets_from_mesh(&mesh, source.clone())
    }

    /// CPU bake from an in-memory mesh (tests and procedural sources).
    pub fn bake_meshlets_from_mesh(
        &mut self,
        mesh: &Mesh,
        source: PathBuf,
    ) -> anyhow::Result<MeshletBakeOutcome> {
        let lod = LodChainConfig::default();
        let key = vg_cache::cache_key(mesh, &lod, 0);
        let asset = build_vg_asset(mesh, &lod, 0);
        let meshlet_count = asset.meshlets.len();
        if meshlet_count == 0 {
            anyhow::bail!("build_vg_asset produced zero meshlets for {}", source.display());
        }
        let tri_count = asset.triangle_count();
        vg_cache::store_cached(key, &asset);

        let cache_path = PathBuf::from(".cache")
            .join("vg")
            .join(format!("{key:016x}.rmesh"));
        let report_path = PathBuf::from(".cache/bake_meshlets.txt");
        std::fs::create_dir_all(".cache")?;
        let gpu_register_line =
            "gpu_register: pending RenderHost (run editor with viewport GPU)".to_string();
        let body = format!(
            "# Meshlet Bake\n\
             generated_at: {}\n\
             source: {}\n\
             api: redox_render::virtual_geometry::build_vg_asset\n\
             cache_api: redox_render::virtual_geometry::cache::store_cached\n\
             cache_key: {key:016x}\n\
             cache_path: {}\n\
             meshlets: {meshlet_count}\n\
             triangles: {tri_count}\n\
             lod_levels: {}\n\
             {gpu_register_line}\n",
            unix_timestamp(),
            source.display(),
            cache_path.display(),
            lod.levels.len(),
        );
        std::fs::write(&report_path, &body)?;

        let msg = format!(
            "Meshlets OK: {meshlet_count} from {} → {}",
            source.display(),
            cache_path.display()
        );
        self.bake_status.meshlets = msg;
        Ok(MeshletBakeOutcome {
            cache_key: key,
            cache_path,
            source,
            meshlet_count,
            gpu_register_line,
        })
    }

    /// Updates the bake report after GPU registration.
    pub fn finalize_meshlet_gpu_register(&mut self, outcome: &MeshletBakeOutcome) -> anyhow::Result<()> {
        let report_path = PathBuf::from(".cache/bake_meshlets.txt");
        if report_path.is_file() {
            let mut body = std::fs::read_to_string(&report_path)?;
            if let Some(line) = body.lines().find(|l| l.starts_with("gpu_register:")) {
                body = body.replace(line, &outcome.gpu_register_line);
            } else {
                body.push_str(&outcome.gpu_register_line);
                body.push('\n');
            }
            std::fs::write(&report_path, body)?;
        }
        if !outcome.gpu_register_line.contains("ok") {
            return Ok(());
        }
        self.bake_status.meshlets = format!(
            "Meshlets OK: {} → {} ({})",
            outcome.meshlet_count,
            outcome.cache_path.display(),
            outcome.gpu_register_line.trim()
        );
        Ok(())
    }

    /// CPU SDF bake via [`bake_mesh`]. GPU atlas upload needs Renderer.
    pub fn bake_sdf(&mut self) -> anyhow::Result<String> {
        let mesh = create_cube();
        let world_size = Vec3::splat(2.0);
        let field = bake_mesh(&mesh, world_size);

        let mut min_d = f32::MAX;
        let mut max_d = f32::MIN;
        for &d in &field.data {
            min_d = min_d.min(d);
            max_d = max_d.max(d);
        }

        let mut atlas = SdfBrickAtlas::new();
        atlas.insert(
            1,
            SdfBrick {
                field: field.clone(),
                origin_ws: Vec3::ZERO,
                world_size,
            },
        );

        let out_dir = PathBuf::from(".cache/sdf");
        std::fs::create_dir_all(&out_dir)?;
        let bin_path = out_dir.join("cube_64.bin");
        let mut bytes = Vec::with_capacity(field.data.len() * 4);
        for v in &field.data {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&bin_path, &bytes)?;

        let report_path = PathBuf::from(".cache/bake_sdf.txt");
        let body = format!(
            "# SDF Bake\n\
             generated_at: {}\n\
             source: procedural unit cube (OFPA actors have no mesh paths)\napi: redox_render::sdf::bake_mesh\n\
             atlas_api: redox_render::sdf::SdfBrickAtlas::insert\n\
             resolution: 64^3\n\
             world_size: {}\n\
             samples: {}\n\
             distance_min: {min_d}\n\
             distance_max: {max_d}\n\
             bricks: {}\n\
             field_bin: {}\n\
             gpu_atlas: FAIL needs SdfGpuAtlas + Renderer queue (scene GPU)\n",
            unix_timestamp(),
            world_size.x,
            field.data.len(),
            atlas.len(),
            bin_path.display(),
        );
        std::fs::write(&report_path, &body)?;

        let msg = format!("SDF OK: 64³ → {} (GPU atlas deferred)", bin_path.display());
        self.bake_status.sdf = msg.clone();
        Ok(msg)
    }

    /// Copy world + bake report + checklist into `.cache/packages/<name>/`.
    pub fn export_package(&mut self) -> anyhow::Result<String> {
        let name = if self.package_name.trim().is_empty() {
            "package"
        } else {
            self.package_name.trim()
        };
        let pkg = PathBuf::from(".cache/packages").join(name);
        if pkg.exists() {
            std::fs::remove_dir_all(&pkg)?;
        }
        std::fs::create_dir_all(&pkg)?;

        let root = Path::new(&self.world_root);
        if root.is_dir() {
            copy_dir_recursive(root, &pkg)?;
        } else {
            anyhow::bail!("world root missing: {}", root.display());
        }

        let report = write_bake_report(root, &self.bake_status)?;
        std::fs::copy(&report.report_path, pkg.join("bake_report.txt"))?;
        self.last_report.replace(report);

        let checklist = format!(
            "# RedOx Package: {name}\n\n\
             generated_at: {}\n\
             world_root: {}\n\n\
             ## Contents\n\
             - OFPA world files (world.ron + cells/)\n\
             - bake_report.txt\n\
             - PACKAGE.md (this file)\n\n\
             ## Bake status\n\
             - Meshlets: {}\n\
             - SDF: {}\n\n\
             ## Ship checklist\n\
             - [ ] OFPA cells scanned\n\
             - [ ] Meshlets baked (CPU cache under .cache/vg/)\n\
             - [ ] SDF baked (CPU field under .cache/sdf/)\n\
             - [ ] GPU atlas / VG register at runtime (Renderer)\n\
             - [ ] Playtest package world root\n",
            unix_timestamp(),
            root.display(),
            if self.bake_status.meshlets.is_empty() {
                "(not run)"
            } else {
                &self.bake_status.meshlets
            },
            if self.bake_status.sdf.is_empty() {
                "(not run)"
            } else {
                &self.bake_status.sdf
            },
        );
        std::fs::write(pkg.join("PACKAGE.md"), checklist)?;

        let msg = format!("Exported package → {}", pkg.display());
        self.status = msg.clone();
        Ok(msg)
    }
}

fn scan_ofpa(root: &Path) -> anyhow::Result<BakeReport> {
    let mut lines = vec![format!("world: {}", root.display())];
    let mut cell_count = 0usize;
    let mut actor_files = 0usize;

    match load_manifest(root) {
        Ok(m) => {
            lines.push(format!(
                "manifest: name={} cell_size={} seed={} listed_cells={}",
                m.name,
                m.cell_size,
                m.seed,
                m.cells.len()
            ));
        }
        Err(e) => lines.push(format!("manifest: (unavailable: {e})")),
    }

    let cells = root.join(CELLS_DIR);
    if !cells.is_dir() {
        lines.push(format!("(missing cells dir: {})", cells.display()));
    } else {
        let mut dirs: Vec<_> = std::fs::read_dir(&cells)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();
        dirs.sort_by_key(|e| e.file_name());

        for entry in dirs {
            cell_count += 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            let parsed = parse_cell_dir(&name);

            let n_actors = if let Some(cell) = parsed {
                match load_cell_actors(root, cell) {
                    Ok(actors) => {
                        actor_files += actors.len();
                        actors.len()
                    }
                    Err(_) => {
                        let n = count_ron_files(&entry.path())?;
                        actor_files += n;
                        n
                    }
                }
            } else {
                let n = count_ron_files(&entry.path())?;
                actor_files += n;
                n
            };

            match parsed {
                Some(c) => lines.push(format!(
                    "cell {}_{} ({}): {n_actors} actors",
                    c.x, c.y, name
                )),
                None => lines.push(format!("cell {name}: {n_actors} actors (unparsed)")),
            }
        }
    }

    let report_path = PathBuf::from(".cache/bake_scan.txt");
    Ok(BakeReport {
        world_root: root.to_path_buf(),
        cell_count,
        actor_files,
        report_path,
        lines,
    })
}

fn count_ron_files(dir: &Path) -> anyhow::Result<usize> {
    let mut n = 0usize;
    for f in std::fs::read_dir(dir)? {
        let f = f?;
        if f.path().extension().and_then(|e| e.to_str()) == Some("ron") {
            n += 1;
        }
    }
    Ok(n)
}

fn write_bake_report(root: &Path, bake: &BakeStatus) -> anyhow::Result<BakeReport> {
    let mut report = scan_ofpa(root)?;
    std::fs::create_dir_all(".cache")?;
    let path = PathBuf::from(".cache/bake_report.txt");
    let mut body = String::new();
    body.push_str("# RedOx Bake Report\n");
    body.push_str(&format!("generated_at: {}\n", unix_timestamp()));
    for line in &report.lines {
        body.push_str(line);
        body.push('\n');
    }
    body.push_str("\n# Bake hooks\n");
    body.push_str(&format!(
        "meshlets: {}\n",
        if bake.meshlets.is_empty() {
            "not run"
        } else {
            &bake.meshlets
        }
    ));
    body.push_str(&format!(
        "sdf: {}\n",
        if bake.sdf.is_empty() {
            "not run"
        } else {
            &bake.sdf
        }
    ));
    body.push_str("\n# Still deferred (scene GPU)\n");
    body.push_str("- VGSystem::register_asset / meshlet GPU upload\n");
    body.push_str("- SdfGpuAtlas brick upload\n");
    body.push_str("- IBL / GI probes\n");
    std::fs::write(&path, body)?;
    report.report_path = path;
    report.lines.insert(0, "bake_report written".into());
    Ok(report)
}

/// `into_world`: write via [`generate_to`] into `root`; otherwise preview under `.cache/pcg_preview`.
fn bake_pcg_arena(root: &Path, into_world: bool) -> anyhow::Result<String> {
    let params = PcgParams {
        name: root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("arena")
            .to_string(),
        kind: PcgKind::Arena,
        width: 4,
        height: 4,
        cell_size: 64.0,
        seed: 42,
    };
    let out = if into_world {
        root.to_path_buf()
    } else {
        PathBuf::from(".cache/pcg_preview")
    };
    std::fs::create_dir_all(&out)?;
    let generated = generate_to(&out, &params).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(format!(
        "PCG wrote {} cells → {} ({})",
        generated.cells.len(),
        out.display(),
        if into_world { "world root" } else { "preview" }
    ))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_file() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Build timestamp as `unix:<seconds>` (no chrono dependency).
/// Loads the first mesh primitive from a glTF/GLB path for meshlet clustering.
pub fn load_mesh_for_vg(path: &Path) -> anyhow::Result<Mesh> {
    let scene = load_scene_from_path(path)?;
    let mesh_idx = scene.nodes.iter().find_map(|n| n.mesh_index).unwrap_or(0);
    let data = scene
        .meshes
        .get(mesh_idx)
        .ok_or_else(|| anyhow::anyhow!("{path:?} has no meshes"))?;
    Ok(redox_editor_viewport::gltf_mesh_to_render_mesh(data))
}

fn unix_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}
