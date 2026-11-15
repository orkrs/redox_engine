//! Optional renderer experiments for Courier: TSR upscaling and virtual geometry (VG-02 DAG).

use redox_math::{Mat4, Quat, Vec3};
use redox_render::context::RenderContext;
use redox_render::mesh::primitive;
use redox_render::virtual_geometry::{
    DagParams, VGConfig, VGInstanceHandle, build_vg_dag_asset,
};

/// Courier CLI / env switches for the renderer experiments.
#[derive(Clone, Debug)]
pub struct ExperimentFlags {
    /// Temporal upscaling (PST-07): TAA stays on, internal resolution is `tsr_scale` of the window.
    pub tsr: bool,
    /// Internal resolution scale when [`Self::tsr`] is true (typical 0.67 or 0.5).
    pub tsr_scale: f32,
    /// Draw ground and crates through the VG visibility path (DAG); hero and coins stay classic.
    pub vg_world: bool,
}

impl ExperimentFlags {
    /// Parses `--tsr`, `--tsr-scale <f32>`, `--vg` and `REDOX_TSR_SCALE` (implies TSR).
    pub fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let tsr = args.iter().any(|a| a == "--tsr");
        let mut tsr_scale = 0.67_f32;
        for (i, a) in args.iter().enumerate() {
            if a == "--tsr-scale" {
                if let Some(v) = args.get(i + 1) {
                    if let Ok(s) = v.parse() {
                        tsr_scale = s;
                    }
                }
            }
        }
        if let Ok(v) = std::env::var("REDOX_TSR_SCALE") {
            if let Ok(s) = v.parse::<f32>() {
                return Self {
                    tsr: true,
                    tsr_scale: s,
                    vg_world: args.iter().any(|a| a == "--vg"),
                };
            }
        }
        Self {
            tsr,
            tsr_scale,
            vg_world: args.iter().any(|a| a == "--vg"),
        }
    }
}

/// Applies TSR settings. TSR is not a separate pass: it is TAA + jitter/LOD/clip tweaks when
/// [`RenderContext::uses_tsr`] is true (`taa_enabled` and `render_scale < 0.999`).
pub fn apply_tsr(ctx: &mut RenderContext, flags: &ExperimentFlags) {
    if !flags.tsr {
        return;
    }
    let scale = flags.tsr_scale.clamp(0.5, 1.0);
    ctx.set_render_scale(scale);
    if !ctx.taa_enabled {
        ctx.enable_taa(true);
        log::info!("TSR needs TAA; temporal antialiasing was turned on");
    }
    log::info!(
        "TSR on: internal {scale:.2}× window, phases={}, lod_bias={:.2}, uses_tsr={}",
        redox_render::post::tsr::jitter_phase_count(scale),
        redox_render::post::tsr::texture_lod_bias(scale),
        ctx.uses_tsr(),
    );
}

/// Runtime VG instances for the static level pieces.
pub struct VgWorld {
    _ground: VGInstanceHandle,
    _crates: Vec<VGInstanceHandle>,
}

impl VgWorld {
    /// Registers cube DAG assets and spawns ground + crates. Call once after materials exist.
    pub fn install(
        ctx: &mut RenderContext,
        level_half: f32,
        crates: &[crate::game::Crate],
        ground_material: u32,
        crate_material: u32,
    ) -> Option<Self> {
        ctx.init_virtual_geometry(VGConfig {
            dag_traverse: true,
            ..VGConfig::default()
        });
        ctx.vg_use_vis_path = true;
        let queue = ctx.queue.clone();
        let mesh = primitive::create_cube();
        let vg = ctx.vg_system.as_mut()?;
        let ground_asset = build_vg_dag_asset(&mesh, &DagParams::default(), ground_material)
            .map_err(|e| {
                log::error!("VG ground bake failed: {e}");
            })
            .ok()?;
        let crate_asset = build_vg_dag_asset(&mesh, &DagParams::default(), crate_material)
            .map_err(|e| {
                log::error!("VG crate bake failed: {e}");
            })
            .ok()?;
        let ground_id = vg.register_dag_asset(ground_asset, &queue)?;
        let crate_id = vg.register_dag_asset(crate_asset, &queue)?;
        let ground = Mat4::from_scale_rotation_translation(
            Vec3::new(level_half * 2.0, 0.25, level_half * 2.0),
            Quat::IDENTITY,
            Vec3::new(0.0, -0.125, 0.0),
        );
        let ground = vg.spawn_dag_instance(ground_id, ground.to_cols_array_2d())?;
        let mut crate_handles = Vec::with_capacity(crates.len());
        for c in crates {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::splat(c.half * 2.0),
                Quat::IDENTITY,
                Vec3::new(c.x, c.half, c.z),
            );
            crate_handles.push(vg.spawn_dag_instance(crate_id, model.to_cols_array_2d())?);
        }
        log::info!(
            "VG world: visibility path on, {} crate instances, materials are solid (streamed albedo stays on classic path when VG is off)",
            crate_handles.len()
        );
        Some(Self {
            _ground: ground,
            _crates: crate_handles,
        })
    }

    /// CPU/GPU cull for DAG instances; call each frame before `render_frame_view`.
    pub fn prepare_frame(ctx: &mut RenderContext) {
        let vp = ctx.camera_uniform.view_proj;
        let cam_pos = Vec3::from_array([
            ctx.camera_uniform.camera_pos[0],
            ctx.camera_uniform.camera_pos[1],
            ctx.camera_uniform.camera_pos[2],
        ]);
        let fov_y = 2.0 * (1.0 / ctx.camera_uniform.proj[1][1]).atan();
        let queue = ctx.queue.clone();
        let Some(vg) = ctx.vg_system.as_mut() else {
            return;
        };
        vg.prepare_frame(&queue, &vp, cam_pos, fov_y);
    }
}

/// When VG draws the static level, skip duplicate classic draws for ground and crates.
pub fn skip_classic_static(flags: &ExperimentFlags) -> bool {
    flags.vg_world
}

/// Logs VG stats about once per second when enabled.
pub fn maybe_log_vg_stats(ctx: &RenderContext, frame: u64) {
    if frame.is_multiple_of(120) {
        if let Some(vg) = ctx.vg_system.as_ref() {
            let s = vg.stats();
            log::info!(
                "VG stats: instances={} visible_tris={} meshlets={}/{}",
                s.active_instances,
                s.visible_triangles,
                s.visible_meshlets,
                s.total_meshlets,
            );
        }
    }
}