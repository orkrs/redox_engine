//! Shader modules of material graph variants (MAT-02 step 5).
//!
//! [`MaterialShaderCache`] keeps one `wgpu::ShaderModule` per [`ShaderKey`]. Every new module is
//! checked with naga before it reaches the device, so a broken graph is reported as an error and
//! the modules already in the cache stay untouched. The caller keeps the key it used before.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use redox_matgraph::{Generated, ShaderKey};

use crate::shader::{create_shader_module, shader_with_vertex_decode};

/// Why a variant module was not built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantError {
    /// naga refused the WGSL. Holds the parser or validator message.
    InvalidWgsl(String),
}

impl std::fmt::Display for VariantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWgsl(message) => write!(f, "material graph WGSL is invalid: {message}"),
        }
    }
}

impl std::error::Error for VariantError {}

/// Full module source of one variant: `base` (the pass shader, e.g. `PBR_WGSL`) followed by the
/// generated `MaterialParams` struct and `material_surface_graph` function.
pub fn variant_source(base: &str, generated: &Generated) -> String {
    shader_with_vertex_decode(&format!(
        "{base}\n{}\n{}\n",
        generated.uniform_wgsl, generated.wgsl
    ))
}

/// Checks WGSL with naga: parse, then validate with all capabilities.
pub fn validate_wgsl(source: &str) -> Result<(), VariantError> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| VariantError::InvalidWgsl(e.emit_to_string(source)))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| VariantError::InvalidWgsl(format!("{e:?}")))
}

/// Shader modules of material graph variants, keyed by [`ShaderKey`].
#[derive(Default)]
pub struct MaterialShaderCache {
    modules: HashMap<ShaderKey, wgpu::ShaderModule>,
}

impl MaterialShaderCache {
    /// Empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of modules held.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// `true` when no module is held.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// `true` when a module for `key` exists.
    pub fn contains(&self, key: ShaderKey) -> bool {
        self.modules.contains_key(&key)
    }

    /// Returns the module for `key`. On the first request `source` is called, the text is
    /// validated and a module is created; later requests reuse the module without calling it.
    /// A source that naga refuses leaves the cache unchanged.
    pub fn get_or_build(
        &mut self,
        device: &wgpu::Device,
        key: ShaderKey,
        label: &str,
        source: impl FnOnce() -> String,
    ) -> Result<&wgpu::ShaderModule, VariantError> {
        match self.modules.entry(key) {
            Entry::Occupied(entry) => Ok(&*entry.into_mut()),
            Entry::Vacant(entry) => {
                let text = source();
                validate_wgsl(&text)?;
                Ok(&*entry.insert(create_shader_module(device, label, &text)))
            }
        }
    }
}
