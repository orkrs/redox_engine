//! Change detection for the on-disk WGSL sources.
//!
//! [`ShaderWatcher`] polls file modification times (std only, no OS watcher) and, given a
//! device, compiles every changed shader inside a validation error scope so a broken edit
//! is reported the moment it is saved.
//!
//! Pipelines are **not** rebuilt here: a passing report means a restart will pick the shader
//! up (with the `shader-hot-reload` feature, modules are read from disk at startup).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{EMBEDDED, needs_vertex_decode, shader_with_vertex_decode};

/// Result of re-compiling one changed shader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderReloadReport {
    /// Shader file stem (e.g. `"pbr"`).
    pub name: String,
    /// `None` when the shader compiled cleanly, otherwise the wgpu validation message.
    pub error: Option<String>,
}

/// Polls `shaders/` for edits.
pub struct ShaderWatcher {
    dir: PathBuf,
    mtimes: HashMap<&'static str, SystemTime>,
}

impl ShaderWatcher {
    /// Watches the crate's own `shaders/` directory; edits made after this call are reported.
    pub fn new() -> Self {
        Self::with_dir(super::shaders_dir())
    }

    /// Watches `dir` (laid out like `shaders/`). Current mtimes become the baseline.
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let mtimes = EMBEDDED
            .iter()
            .filter_map(|(stem, rel, _)| modified(&dir.join(rel)).map(|t| (*stem, t)))
            .collect();
        Self { dir, mtimes }
    }

    /// Stems of shaders modified (or created) since the previous call. File system only.
    pub fn changed(&mut self) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (stem, rel, _) in EMBEDDED {
            let Some(now) = modified(&self.dir.join(rel)) else {
                continue;
            };
            if self.mtimes.insert(stem, now) != Some(now) {
                out.push(*stem);
            }
        }
        out
    }

    /// Re-compiles each changed shader on `device` and returns one report per shader.
    pub fn poll(&mut self, device: &wgpu::Device) -> Vec<ShaderReloadReport> {
        self.changed()
            .into_iter()
            .map(|stem| {
                let report = self.validate(device, stem);
                match &report.error {
                    None => log::info!("shader '{stem}' recompiled OK (restart to apply)"),
                    Some(e) => log::error!("shader '{stem}' failed to compile: {e}"),
                }
                report
            })
            .collect()
    }

    fn validate(&self, device: &wgpu::Device, stem: &'static str) -> ShaderReloadReport {
        let error = match self.read(stem) {
            Err(e) => Some(e),
            Ok(src) => compile_error(device, stem, &src),
        };
        ShaderReloadReport {
            name: stem.to_string(),
            error,
        }
    }

    fn read(&self, stem: &str) -> Result<String, String> {
        let rel = EMBEDDED
            .iter()
            .find(|(s, _, _)| *s == stem)
            .map(|(_, rel, _)| *rel)
            .ok_or_else(|| format!("unknown shader '{stem}'"))?;
        std::fs::read_to_string(self.dir.join(rel)).map_err(|e| format!("cannot read {rel}: {e}"))
    }
}

impl Default for ShaderWatcher {
    fn default() -> Self {
        Self::new()
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn compile_error(device: &wgpu::Device, stem: &str, src: &str) -> Option<String> {
    let full = if needs_vertex_decode(stem) {
        shader_with_vertex_decode(src)
    } else {
        src.to_string()
    };
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(stem),
        source: wgpu::ShaderSource::Wgsl(full.into()),
    });
    pollster::block_on(device.pop_error_scope()).map(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn temp_shader_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("redox_shader_watch_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("smaa")).unwrap();
        dir
    }

    fn touch(path: &Path, offset_secs: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(offset_secs))
            .unwrap();
    }

    #[test]
    fn reports_only_modified_shaders() {
        let dir = temp_shader_dir("modified");
        fs::write(dir.join("pbr.wgsl"), "// a").unwrap();
        fs::write(dir.join("bloom.wgsl"), "// b").unwrap();

        let mut watcher = ShaderWatcher::with_dir(&dir);
        assert!(
            watcher.changed().is_empty(),
            "baseline must not report anything"
        );

        touch(&dir.join("pbr.wgsl"), 5);
        assert_eq!(watcher.changed(), vec!["pbr"]);
        assert!(watcher.changed().is_empty(), "a change is reported once");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn newly_created_shader_counts_as_changed() {
        let dir = temp_shader_dir("created");
        let mut watcher = ShaderWatcher::with_dir(&dir);
        fs::write(dir.join("smaa").join("edge.wgsl"), "// new").unwrap();
        assert_eq!(watcher.changed(), vec!["smaa_edge"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_directory_is_quiet() {
        let mut watcher =
            ShaderWatcher::with_dir(std::env::temp_dir().join("redox_no_such_shader_dir"));
        assert!(watcher.changed().is_empty());
    }
}
