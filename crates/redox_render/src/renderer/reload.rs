//! Rebuilding pipelines after a shader edit, without restarting.
//!
//! Pass objects only depend on the device and a few formats, and wgpu treats bind group layouts
//! with equal entries as compatible, so existing bind groups keep working with a rebuilt
//! pipeline. With the `shader-hot-reload` feature the new pipelines compile the edited files
//! from disk; without it they are rebuilt from the embedded sources (no visible change).

use super::*;

/// Passes that [`Renderer::reload_shaders`] can rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReloadablePass {
    /// Forward PBR and the transparent pass built on it.
    Pbr,
    /// Directional shadow depth.
    Shadow,
    /// Normals prepass.
    Normal,
    /// GTAO trace, blur and upsample.
    Ssao,
    /// Screen-space VSM shadow trace and temporal filter.
    VsmShadow,
    /// ScatterLight point-light path.
    ScatterLight,
    /// Bloom.
    Bloom,
    /// MSAA resolve.
    MsaaResolve,
    /// TAA resolve.
    Taa,
    /// Velocity.
    Velocity,
    /// Debug lines.
    DebugLines,
}

impl ReloadablePass {
    /// The pass built from the shader with file stem `stem`, if it can be rebuilt.
    pub fn for_shader(stem: &str) -> Option<Self> {
        Some(match stem {
            "pbr" | "forward" => Self::Pbr,
            "shadow" => Self::Shadow,
            "normal" => Self::Normal,
            "gtao" | "gtao_blur" | "gtao_upsample" => Self::Ssao,
            "vsm_shadow_trace" | "vsm_shadow_temporal" => Self::VsmShadow,
            "scatter_light" => Self::ScatterLight,
            "bloom" | "bloom_composite" => Self::Bloom,
            "msaa_resolve" => Self::MsaaResolve,
            "taa" => Self::Taa,
            "velocity" => Self::Velocity,
            "debug_lines" => Self::DebugLines,
            _ => return None,
        })
    }
}

impl Renderer {
    /// Rebuilds the passes behind the shaders in `stems` (file stems such as `"pbr"`, as reported
    /// by [`ShaderWatcher`](crate::shader::watch::ShaderWatcher)). Returns the passes that were
    /// rebuilt; unknown stems and shaders of passes that cannot be rebuilt are skipped.
    ///
    /// A broken shader makes wgpu report a validation error as usual, so check the file with
    /// `ShaderWatcher::poll` first and only reload shaders that compile.
    pub fn reload_shaders(&mut self, stems: &[&str]) -> Vec<ReloadablePass> {
        let mut passes: Vec<ReloadablePass> = stems
            .iter()
            .filter_map(|s| ReloadablePass::for_shader(s))
            .collect();
        passes.sort_by_key(|p| *p as u8);
        passes.dedup();

        let hdr = wgpu::TextureFormat::Rgba16Float;
        let msaa = self.msaa_sample_count;
        for pass in &passes {
            match pass {
                ReloadablePass::Pbr => {
                    self.pbr_pass = PbrPass::new(&self.device, hdr, msaa);
                    self.transparent_pass =
                        TransparentPass::new(&self.device, &self.pbr_pass, hdr, msaa);
                }
                ReloadablePass::Shadow => self.shadow_pass = ShadowPass::new(&self.device),
                ReloadablePass::Normal => {
                    self.normal_pass =
                        NormalPass::new(&self.device, wgpu::TextureFormat::Rgba16Float);
                }
                ReloadablePass::Ssao => self.ssao_pass = SSAOPass::new(&self.device, &self.queue),
                ReloadablePass::VsmShadow => {
                    self.vsm_shadow_pass = VsmShadowPass::new(&self.device)
                }
                ReloadablePass::ScatterLight => {
                    self.scatter_light_pass = ScatterLightPass::new(&self.device, hdr);
                }
                ReloadablePass::Bloom => self.bloom_pass = BloomPass::new(&self.device, hdr),
                ReloadablePass::MsaaResolve => {
                    self.msaa_resolve_pass = MsaaResolvePass::new(&self.device, hdr);
                    self.msaa_resolve_pass.set_sample_count(&self.queue, msaa);
                }
                ReloadablePass::Taa => self.taa_pass = TaaPass::new(&self.device, &self.queue, hdr),
                ReloadablePass::Velocity => self.velocity_pass = VelocityPass::new(&self.device),
                ReloadablePass::DebugLines => {
                    self.debug_lines_pass =
                        DebugLinesPass::new(&self.device, &self.shared_layouts.camera_bgl, msaa);
                }
            }
        }
        if !passes.is_empty() {
            log::info!("rebuilt passes after shader edit: {passes:?}");
        }
        passes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_stems_map_to_passes() {
        assert_eq!(ReloadablePass::for_shader("pbr"), Some(ReloadablePass::Pbr));
        assert_eq!(
            ReloadablePass::for_shader("gtao_blur"),
            Some(ReloadablePass::Ssao)
        );
        assert_eq!(ReloadablePass::for_shader("vg_cull"), None);
    }
}
