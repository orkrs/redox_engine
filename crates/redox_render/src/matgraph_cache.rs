//! Shader modules and draw state of material graph variants (MAT-02 steps 5).
//!
//! [`MaterialShaderCache`] keeps one `wgpu::ShaderModule` per [`ShaderKey`]. Every new module is
//! checked with naga before it reaches the device, so a broken graph is reported as an error and
//! the modules already in the cache stay untouched.
//!
//! [`MaterialGraphs`] binds graph variants to material slots: each slot gets one variant per
//! pass (forward, alpha and additive transparent, depth prepass, shadow atlas) and its own
//! parameter block. Slots without a graph are never touched and keep the default pipelines.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::PathBuf;

use redox_matgraph::{
    CodegenOptions, GRAPH_TEXTURE_SLOTS, Generated, GraphError, GraphReload, MatGraphWatcher,
    MaterialGraph, ShaderKey, Variant, compile_variant,
};

use crate::matgraph_passes::{
    GraphPass, PassBindLayouts, blended_pipeline, depth_pipeline, depth_source, shadow_pipeline,
    shadow_source, vsm_pipeline, vsm_source,
};
use crate::pass::pbr::{PbrPass, pbr_render_pipeline};
use crate::pass::transparent::ADDITIVE_BLEND;
use crate::shader::{PBR_WGSL, create_shader_module, shader_with_vertex_decode};

/// Bind group index of the graph parameter block, after the four PBR groups.
pub const GRAPH_PARAMS_GROUP: u32 = 4;

/// The call to `material_surface` in the fragment entry of `pbr.wgsl`, which graph variants
/// replace.
const SURFACE_CALL: &str = "let surf = material_surface(";

/// Replacement call. Time is the frame index at 60 Hz (ASSUMPTION: no time uniform in the
/// forward pass yet).
const GRAPH_SURFACE_CALL: &str =
    "let surf = material_surface_graph_variant(in.uv, mat_graph_time.x, ";

/// The world position line of `vs_main` in `pbr.wgsl`, which graph variants extend with the
/// vertex offset of the graph.
const VERTEX_POSITION_LINE: &str = "let world_pos = model * vec4<f32>(local_pos, 1.0);";

/// Replacement: the same position plus `material_vertex_offset_graph`.
const GRAPH_VERTEX_POSITION: &str = "var world_pos = model * vec4<f32>(local_pos, 1.0);
    {
        let wpo_n = normalize(mdl.normal_matrix * oct_decode(in.normal_oct));
        let wpo_in = SurfaceInput(world_pos.xyz, wpo_n, vec3<f32>(1.0, 0.0, 0.0), mdl.color, camera.camera_pos.xyz - world_pos.xyz);
        world_pos = world_pos + vec4<f32>(material_vertex_offset_graph(wpo_in, in.uv, mat_graph_time.x, mat_graph_params), 0.0);
    }";

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

/// Why a graph could not be bound to a material slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialGraphError {
    /// The graph failed checking; holds every error found, each naming its node.
    Graph(Vec<GraphError>),
    /// The generated WGSL was refused.
    Wgsl(VariantError),
    /// A material instance names a graph that is not registered.
    UnknownGraph(crate::material::MaterialGraphId),
}

impl std::fmt::Display for MaterialGraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Graph(errors) => {
                let text: Vec<String> = errors.iter().map(ToString::to_string).collect();
                write!(f, "material graph is invalid: {}", text.join("; "))
            }
            Self::Wgsl(error) => write!(f, "{error}"),
            Self::UnknownGraph(id) => write!(f, "unknown material graph {}", id.0),
        }
    }
}

impl std::error::Error for MaterialGraphError {}

/// Full module source of one variant for a compute or test shader: `base` followed by the
/// generated `MaterialParams` struct and `material_surface_graph` function.
pub fn variant_source(base: &str, generated: &Generated) -> String {
    shader_with_vertex_decode(&format!(
        "{base}\n{}\n{}\n",
        generated.uniform_wgsl, generated.wgsl
    ))
}

/// Forward-pass source of a graph variant: `base` (`PBR_WGSL`) with its `material_surface` call
/// replaced by `material_surface_graph_variant`, which reads the parameter block in group 4.
///
/// The replacement is a text edit on the known call site; a base without that call is refused.
pub fn graph_pass_source(base: &str, generated: &Generated) -> Result<String, VariantError> {
    if !base.contains(SURFACE_CALL) {
        return Err(VariantError::InvalidWgsl(
            "pass shader has no material_surface call".to_string(),
        ));
    }
    let body = base.replacen(SURFACE_CALL, GRAPH_SURFACE_CALL, 1).replacen(
        VERTEX_POSITION_LINE,
        GRAPH_VERTEX_POSITION,
        1,
    );
    Ok(shader_with_vertex_decode(&format!(
        "{body}\n{}\n{}\n\
         @group({GRAPH_PARAMS_GROUP}) @binding(0) var<uniform> mat_graph_params: MaterialParams;\n\
         @group({GRAPH_PARAMS_GROUP}) @binding(1) var<uniform> mat_graph_time: vec4<f32>;\n\
         fn material_surface_graph_variant(uv: vec2<f32>, time: f32, surface_in: SurfaceInput, \
         mat: Material, tex: TextureSamples) -> SurfaceData {{\n\
         \x20   return material_surface_graph(surface_in, uv, time, mat_graph_params);\n\
         }}\n",
        generated.uniform_wgsl, generated.wgsl
    )))
}

/// Version of the disk cache file layout. Bump it when the header or the file naming changes;
/// files written by another version are regenerated.
pub const DISK_CACHE_VERSION: u32 = 1;

/// FNV-1a hash of `bytes`. Stable across runs and processes, unlike the randomly seeded std
/// hasher, so it can be stored in cache files.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

/// Hash of a graph's source text (its canonical RON form). Cache files of a graph carry it.
pub fn graph_source_hash(graph: &MaterialGraph) -> u64 {
    fnv1a(graph.to_ron().as_bytes())
}

/// Text of a cache file: a header line with the cache version, the format version, the graph
/// source hash and the hash of the body, then the body (the module source).
pub fn cache_text(source_hash: u64, body: &str) -> String {
    format!(
        "// matgraph-cache v{DISK_CACHE_VERSION} format={} source={source_hash:016x} body={:016x}\n{body}",
        redox_matgraph::FORMAT_VERSION,
        fnv1a(body.as_bytes()),
    )
}

/// The body of a cache file, if its header names this cache version, this format version and
/// this graph source, and the body still hashes to the recorded value. `None` means the file
/// must be regenerated.
pub fn read_cached_body(text: &str, source_hash: u64) -> Option<&str> {
    let (header, body) = text.split_once('\n')?;
    let prefix = format!(
        "// matgraph-cache v{DISK_CACHE_VERSION} format={} source={source_hash:016x} body=",
        redox_matgraph::FORMAT_VERSION,
    );
    let recorded = u64::from_str_radix(header.strip_prefix(prefix.as_str())?, 16).ok()?;
    (recorded == fnv1a(body.as_bytes())).then_some(body)
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
    hits: u64,
    misses: u64,
}

/// More modules than this in one cache is logged once: the variant count is meant to stay small.
pub const MAX_VARIANTS_WARN: usize = 256;

/// Counters of a [`MaterialShaderCache`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Modules held.
    pub modules: usize,
    /// Requests answered from the cache.
    pub hits: u64,
    /// Requests that built a module.
    pub misses: u64,
}

impl MaterialShaderCache {
    /// Empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Modules held and the hit / miss counters.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            modules: self.modules.len(),
            hits: self.hits,
            misses: self.misses,
        }
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

    /// The module for `key`, if it was built.
    pub fn module(&self, key: ShaderKey) -> Option<&wgpu::ShaderModule> {
        self.modules.get(&key)
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
        if self.modules.len() + 1 == MAX_VARIANTS_WARN && !self.modules.contains_key(&key) {
            log::warn!("material graph cache reaches {MAX_VARIANTS_WARN} shader variants");
        }
        match self.modules.entry(key) {
            Entry::Occupied(entry) => {
                self.hits += 1;
                Ok(&*entry.into_mut())
            }
            Entry::Vacant(entry) => {
                let text = source();
                validate_wgsl(&text)?;
                self.misses += 1;
                Ok(&*entry.insert(create_shader_module(device, label, &text)))
            }
        }
    }
}

/// Graph state of one material slot: the variant key of every pass and its parameter block.
struct GraphMaterial {
    /// Key of the forward module; the alpha and additive passes share it.
    key: ShaderKey,
    /// Key of the depth prepass module.
    depth_key: ShaderKey,
    /// Key of the shadow atlas module.
    shadow_key: ShaderKey,
    /// Key of the sun shadow page module.
    vsm_key: ShaderKey,
    /// Longest vertex offset the graph declares, metres.
    wpo_max: f32,
    /// Keeps the parameter block alive for the bind groups.
    _params_buffer: wgpu::Buffer,
    params: wgpu::BindGroup,
}

impl GraphMaterial {
    /// Module key used by `pass`.
    fn key_for(&self, pass: GraphPass) -> ShaderKey {
        match pass {
            GraphPass::Forward | GraphPass::Alpha | GraphPass::Additive => self.key,
            GraphPass::Depth => self.depth_key,
            GraphPass::Shadow => self.shadow_key,
            GraphPass::VsmDepth => self.vsm_key,
        }
    }
}

/// Graph-driven material slots of the forward, transparent, prepass and shadow passes
/// (MAT-02 step 5).
pub struct MaterialGraphs {
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    graph_bgl: wgpu::BindGroupLayout,
    /// Forward and transparent layout: the four PBR groups followed by the graph parameter
    /// group. `None` when the device allows fewer than five bind groups: graph materials are
    /// refused.
    layout: Option<wgpu::PipelineLayout>,
    /// Depth prepass layout: scene group 0 and the graph parameter group 1.
    depth_layout: wgpu::PipelineLayout,
    /// Shadow atlas layout: shadow matrix group 0, model group 1, graph parameter group 2.
    shadow_layout: wgpu::PipelineLayout,
    /// Sun shadow page layout: page scene group 0 and the graph parameter group 1.
    vsm_layout: wgpu::PipelineLayout,
    /// Frame time uniform bound at binding 1 of every parameter group.
    time_buffer: wgpu::Buffer,
    /// 1x1 white texture bound to the texture slots a material does not fill.
    white: wgpu::TextureView,
    /// Samplers at bindings 6..=8: default, repeat and clamp (see [`GRAPH_SAMPLER_BINDING`]).
    samplers: [wgpu::Sampler; 3],
    cache: MaterialShaderCache,
    pipelines: HashMap<(ShaderKey, GraphPass), wgpu::RenderPipeline>,
    materials: HashMap<usize, GraphMaterial>,
    /// Directory for debug copies of the generated WGSL, written once per variant.
    disk_cache: Option<PathBuf>,
}

impl MaterialGraphs {
    /// Creates the state for the forward pass `pbr` (colour format and MSAA count as in `PbrPass`).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pbr: &PbrPass,
        surface_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let graph_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("matgraph_params_bgl"),
            entries: &graph_bgl_entries(),
        });
        let white = white_view(device, queue);
        let samplers = graph_samplers(device);
        let time_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("matgraph_time"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = graph_layout(device, pbr, &graph_bgl);
        let bind_layouts = PassBindLayouts::new(device);
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("matgraph_depth_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_layouts.depth_scene), Some(&graph_bgl)],
            immediate_size: 0,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("matgraph_shadow_pipeline_layout"),
            bind_group_layouts: &[
                Some(&bind_layouts.shadow_scene),
                Some(&bind_layouts.shadow_model),
                Some(&graph_bgl),
            ],
            immediate_size: 0,
        });
        let vsm_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("matgraph_vsm_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_layouts.vsm_scene), Some(&graph_bgl)],
            immediate_size: 0,
        });
        Self {
            surface_format,
            sample_count,
            graph_bgl,
            time_buffer,
            white,
            samplers,
            layout,
            depth_layout,
            shadow_layout,
            vsm_layout,
            cache: MaterialShaderCache::new(),
            pipelines: HashMap::new(),
            materials: HashMap::new(),
            disk_cache: None,
        }
    }

    /// Sets (or clears, with `None`) the directory for debug copies of generated WGSL.
    pub fn set_disk_cache(&mut self, dir: Option<PathBuf>) {
        self.disk_cache = dir;
    }

    /// Number of shader modules (one per distinct variant).
    pub fn shader_count(&self) -> usize {
        self.cache.len()
    }

    /// Number of render pipelines (one per variant and pass in use).
    pub fn pipeline_count(&self) -> usize {
        self.pipelines.len()
    }

    /// Number of material slots bound to a graph.
    pub fn material_count(&self) -> usize {
        self.materials.len()
    }

    /// Sets the frame time (seconds) every graph material reads as `time`; call once per frame
    /// so the vertex and fragment stages of all passes see the same value.
    pub fn set_time(&self, queue: &wgpu::Queue, seconds: f32) {
        queue.write_buffer(
            &self.time_buffer,
            0,
            bytemuck::cast_slice(&[seconds, 0.0, 0.0, 0.0]),
        );
    }

    /// Longest vertex offset the graph of `material_index` declares (0 without a graph or offset).
    pub fn wpo_max(&self, material_index: usize) -> f32 {
        self.materials
            .get(&material_index)
            .map_or(0.0, |m| m.wpo_max)
    }

    /// `true` when some graph material has a vertex offset.
    pub fn any_wpo(&self) -> bool {
        self.materials.values().any(|m| m.wpo_max > 0.0)
    }

    /// `true` when the material slot `material_index` is bound to a graph.
    pub fn is_graph(&self, material_index: usize) -> bool {
        self.materials.contains_key(&material_index)
    }

    /// Returns the slot `material_index` to the default pipelines. `true` when it had a graph.
    pub fn clear_material(&mut self, material_index: usize) -> bool {
        self.materials.remove(&material_index).is_some()
    }

    /// Forward pipeline and parameter bind group of a graph-driven material slot.
    pub fn binding(
        &self,
        material_index: usize,
    ) -> Option<(&wgpu::RenderPipeline, &wgpu::BindGroup)> {
        self.pass_binding(material_index, GraphPass::Forward)
    }

    /// Pipeline of `pass` and the parameter bind group of a graph-driven material slot. The
    /// bind group belongs at [`GraphPass::params_group`] of that pass.
    pub fn pass_binding(
        &self,
        material_index: usize,
        pass: GraphPass,
    ) -> Option<(&wgpu::RenderPipeline, &wgpu::BindGroup)> {
        let material = self.materials.get(&material_index)?;
        let pipeline = self.pipelines.get(&(material.key_for(pass), pass))?;
        Some((pipeline, &material.params))
    }

    /// Binds `graph` to the material slot `material_index`, with parameter values by slot
    /// (missing entries use the slot default). Builds the variants of every pass.
    ///
    /// On error nothing changes for the slot: it keeps the graph it had before, or the default
    /// pipelines when it had none.
    pub fn set_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        material_index: usize,
        graph: &MaterialGraph,
        params: &[[f32; 4]],
    ) -> Result<ShaderKey, MaterialGraphError> {
        if self.layout.is_none() {
            return Err(MaterialGraphError::Wgsl(VariantError::InvalidWgsl(
                "the device allows fewer than 5 bind groups".to_owned(),
            )));
        }
        let source_hash = graph_source_hash(graph);
        let (key, forward) = self.ensure_variant(device, graph, source_hash, GraphPass::Forward)?;
        let (depth_key, _) = self.ensure_variant(device, graph, source_hash, GraphPass::Depth)?;
        let (shadow_key, _) = self.ensure_variant(device, graph, source_hash, GraphPass::Shadow)?;
        let (vsm_key, _) = self.ensure_variant(device, graph, source_hash, GraphPass::VsmDepth)?;
        for pass in [GraphPass::Alpha, GraphPass::Additive] {
            self.ensure_pipeline(device, key, pass);
        }

        let values = slot_values(&forward.ir.params, params);
        let bytes = parameter_bytes(&values);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("matgraph_params"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, &bytes);
        let params_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("matgraph_params_bg"),
            layout: &self.graph_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.time_buffer.as_entire_binding(),
                },
            ],
        });
        self.materials.insert(
            material_index,
            GraphMaterial {
                key,
                depth_key,
                shadow_key,
                vsm_key,
                wpo_max: forward.ir.wpo_max,
                _params_buffer: buffer,
                params: params_bg,
            },
        );
        Ok(key)
    }

    /// Compiles `graph` for `pass`, builds its module if missing and its pipeline if missing.
    /// Returns the key and the compiled variant (the forward one carries the parameter layout).
    /// A module missing from memory is taken from the disk cache when its file matches the
    /// graph source and version; otherwise the WGSL is generated and the file rewritten.
    fn ensure_variant(
        &mut self,
        device: &wgpu::Device,
        graph: &MaterialGraph,
        source_hash: u64,
        pass: GraphPass,
    ) -> Result<(ShaderKey, Variant), MaterialGraphError> {
        let options = CodegenOptions {
            pass: pass.codegen(),
            features: pass.features(),
        };
        let variant = compile_variant(graph, &options).map_err(MaterialGraphError::Graph)?;
        let key = variant.key;
        if !self.cache.contains(key) {
            let source = match self.read_disk_copy(key, source_hash) {
                Some(body) => body,
                None => {
                    let body = variant_module(pass, &variant.generated)
                        .map_err(MaterialGraphError::Wgsl)?;
                    self.write_disk_copy(key, source_hash, &body);
                    body
                }
            };
            self.cache
                .get_or_build(device, key, "matgraph_variant", || source)
                .map_err(MaterialGraphError::Wgsl)?;
        }
        self.ensure_pipeline(device, key, pass);
        Ok((key, variant))
    }

    /// Builds the pipeline of (`key`, `pass`) unless it exists.
    fn ensure_pipeline(&mut self, device: &wgpu::Device, key: ShaderKey, pass: GraphPass) {
        if !self.pipelines.contains_key(&(key, pass)) {
            self.build_pipeline(device, key, pass);
        }
    }

    /// Removes the graph of a material slot; the slot goes back to the default pipelines.
    pub fn remove_material(&mut self, material_index: usize) -> bool {
        self.materials.remove(&material_index).is_some()
    }

    /// Rebuilds the pipeline layout and every pipeline after the forward pass was recreated
    /// (resize, reload). Modules stay in the cache.
    pub fn rebuild_pipelines(&mut self, device: &wgpu::Device, pbr: &PbrPass) {
        self.layout = graph_layout(device, pbr, &self.graph_bgl);
        self.pipelines.clear();
        let mut jobs: Vec<(ShaderKey, GraphPass)> = Vec::new();
        for material in self.materials.values() {
            for pass in GraphPass::ALL {
                jobs.push((material.key_for(pass), pass));
            }
        }
        for (key, pass) in jobs {
            self.ensure_pipeline(device, key, pass);
        }
    }

    /// Creates the pipeline of a variant whose module is already in the cache. Forward,
    /// alpha and additive pipelines are skipped while the layout is unavailable.
    fn build_pipeline(&mut self, device: &wgpu::Device, key: ShaderKey, pass: GraphPass) {
        let Some(module) = self.cache.module(key) else {
            return;
        };
        let pipeline = match pass {
            GraphPass::Forward => {
                let Some(layout) = self.layout.as_ref() else {
                    return;
                };
                pbr_render_pipeline(
                    device,
                    layout,
                    module,
                    self.surface_format,
                    self.sample_count,
                    "matgraph_pipeline",
                )
            }
            GraphPass::Alpha | GraphPass::Additive => {
                let Some(layout) = self.layout.as_ref() else {
                    return;
                };
                let (blend, label) = if pass == GraphPass::Alpha {
                    (wgpu::BlendState::ALPHA_BLENDING, "matgraph_alpha_pipeline")
                } else {
                    (ADDITIVE_BLEND, "matgraph_additive_pipeline")
                };
                blended_pipeline(
                    device,
                    layout,
                    module,
                    self.surface_format,
                    self.sample_count,
                    blend,
                    label,
                )
            }
            GraphPass::Depth => depth_pipeline(device, &self.depth_layout, module),
            GraphPass::Shadow => shadow_pipeline(device, &self.shadow_layout, module),
            GraphPass::VsmDepth => vsm_pipeline(device, &self.vsm_layout, module),
        };
        self.pipelines.insert((key, pass), pipeline);
    }

    /// Writes the WGSL of a new variant to the disk cache with its header. Failures are logged,
    /// not fatal.
    fn write_disk_copy(&self, key: ShaderKey, source_hash: u64, body: &str) {
        let Some(dir) = &self.disk_cache else {
            return;
        };
        let text = cache_text(source_hash, body);
        let result = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(dir.join(format!("{:016x}.wgsl", key.0)), text));
        if let Err(error) = result {
            log::warn!(
                "matgraph: could not write WGSL copy to {}: {error}",
                dir.display()
            );
        }
    }

    /// The module body cached for `key`, if the disk cache holds a file that matches the graph
    /// source and version. A missing, unreadable or stale file gives `None`.
    fn read_disk_copy(&self, key: ShaderKey, source_hash: u64) -> Option<String> {
        let dir = self.disk_cache.as_ref()?;
        let text = std::fs::read_to_string(dir.join(format!("{:016x}.wgsl", key.0))).ok()?;
        read_cached_body(&text, source_hash).map(str::to_owned)
    }
}

/// Module source of `variant` for `pass`.
/// First binding of the texture slots (`tex0` at this binding, `tex3` three after it).
pub(crate) const GRAPH_TEXTURE_BINDING: u32 = 2;
/// First binding of the samplers: default at this binding, then repeat and clamp.
pub(crate) const GRAPH_SAMPLER_BINDING: u32 = GRAPH_TEXTURE_BINDING + GRAPH_TEXTURE_SLOTS as u32;

/// Parameter group entries: the parameter block and frame time (both vertex and fragment), then
/// the texture slots and the three samplers (fragment only).
fn graph_bgl_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let sampler = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    let textures = (GRAPH_TEXTURE_BINDING..).take(GRAPH_TEXTURE_SLOTS as usize);
    let samplers = (GRAPH_SAMPLER_BINDING..).take(3);
    let mut entries = vec![uniform(0), uniform(1)];
    entries.extend(textures.map(texture));
    entries.extend(samplers.map(sampler));
    entries
}

/// 1x1 opaque white texture: the value of a texture slot the material leaves empty.
fn white_view(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("matgraph_white"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255, 255, 255, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Linear samplers with the given wrap mode, in binding order: default and repeat wrap, clamp
/// clamps to the edge.
fn graph_samplers(device: &wgpu::Device) -> [wgpu::Sampler; 3] {
    let sampler = |label: &str, address: wgpu::AddressMode| {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        })
    };
    [
        sampler("matgraph_samp_default", wgpu::AddressMode::Repeat),
        sampler("matgraph_samp_repeat", wgpu::AddressMode::Repeat),
        sampler("matgraph_samp_clamp", wgpu::AddressMode::ClampToEdge),
    ]
}

fn variant_module(pass: GraphPass, generated: &Generated) -> Result<String, VariantError> {
    match pass {
        GraphPass::Forward | GraphPass::Alpha | GraphPass::Additive => {
            graph_pass_source(PBR_WGSL, generated)
        }
        GraphPass::Depth => Ok(depth_source(generated)),
        GraphPass::Shadow => Ok(shadow_source(generated)),
        GraphPass::VsmDepth => Ok(vsm_source(generated)),
    }
}

/// Bind groups the graph pass needs: the four PBR groups and the graph parameter group.
pub const GRAPH_BIND_GROUPS: u32 = 5;

/// Pipeline layout of the graph pass: the PBR groups 0..3 and the graph parameter group 4.
fn graph_layout(
    device: &wgpu::Device,
    pbr: &PbrPass,
    graph_bgl: &wgpu::BindGroupLayout,
) -> Option<wgpu::PipelineLayout> {
    if device.limits().max_bind_groups < GRAPH_BIND_GROUPS {
        return None;
    }
    Some(
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("matgraph_pipeline_layout"),
            bind_group_layouts: &[
                Some(&pbr.frame_bgl),
                Some(&pbr.scene_bgl),
                Some(&pbr.material_bgl),
                Some(&pbr.model_bgl),
                Some(graph_bgl),
            ],
            immediate_size: 0,
        }),
    )
}

/// One value per parameter slot: the given value, or the slot default when it is missing.
fn slot_values(slots: &[redox_matgraph::ParamSlot], params: &[[f32; 4]]) -> Vec<[f32; 4]> {
    slots
        .iter()
        .enumerate()
        .map(|(index, slot)| params.get(index).copied().unwrap_or(slot.default))
        .collect()
}

/// Uniform bytes of the parameter block. An empty block is 16 zero bytes, matching the padding
/// member the generated struct declares in that case.
fn parameter_bytes(values: &[[f32; 4]]) -> Vec<u8> {
    if values.is_empty() {
        return vec![0; 16];
    }
    bytemuck::cast_slice(values).to_vec()
}

/// Poll interval of [`MaterialGraphHotReload::new`].
const GRAPH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// A `.matgraph` file bound to a material slot.
struct WatchedGraph {
    material_index: usize,
    params: Vec<[f32; 4]>,
    watcher: MatGraphWatcher,
}

/// One failed reload of a material slot. The slot keeps the graph it had before.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphReloadFailure {
    /// Material slot whose file failed.
    pub material_index: usize,
    /// Why the new graph was not applied.
    pub error: MaterialGraphError,
}

/// Watches `.matgraph` files of material slots and rebuilds a slot's variants when its file
/// changes, without a restart (MAT-02 step 6).
///
/// Call [`Self::update`] once per frame, as the shader hot reload is called. A
/// file that does not parse, validate or compile is reported once per change; the slot keeps
/// its previous graph and pipelines.
pub struct MaterialGraphHotReload {
    watched: Vec<WatchedGraph>,
    interval: std::time::Duration,
    last_poll: Option<std::time::Instant>,
}

impl Default for MaterialGraphHotReload {
    fn default() -> Self {
        Self::new()
    }
}

impl MaterialGraphHotReload {
    /// Polls at most twice a second.
    pub fn new() -> Self {
        Self::with_interval(GRAPH_POLL_INTERVAL)
    }

    /// Polls at most once per `interval` (`Duration::ZERO` polls on every call).
    pub fn with_interval(interval: std::time::Duration) -> Self {
        Self {
            watched: Vec::new(),
            interval,
            last_poll: None,
        }
    }

    /// Watches `path` for material slot `material_index`, whose parameter values are `params`.
    /// The file's current state counts as loaded: only later changes reload.
    pub fn watch(
        &mut self,
        material_index: usize,
        path: impl Into<std::path::PathBuf>,
        params: Vec<[f32; 4]>,
    ) {
        self.unwatch(material_index);
        self.watched.push(WatchedGraph {
            material_index,
            params,
            watcher: MatGraphWatcher::new(path),
        });
    }

    /// Stops watching the slot. Its graph stays bound. Returns `true` if it was watched.
    pub fn unwatch(&mut self, material_index: usize) -> bool {
        let before = self.watched.len();
        self.watched.retain(|w| w.material_index != material_index);
        self.watched.len() != before
    }

    /// Number of watched files.
    pub fn len(&self) -> usize {
        self.watched.len()
    }

    /// `true` when no file is watched.
    pub fn is_empty(&self) -> bool {
        self.watched.is_empty()
    }

    /// Polls the watched files (at most once per interval) and rebuilds the slots whose files
    /// changed and now compile. Returns the failures of this call; they are also logged.
    pub fn update(&mut self, renderer: &mut crate::Renderer) -> Vec<GraphReloadFailure> {
        if self
            .last_poll
            .is_some_and(|last| last.elapsed() < self.interval)
        {
            return Vec::new();
        }
        self.last_poll = Some(std::time::Instant::now());
        let mut failures = Vec::new();
        for watched in &mut self.watched {
            for reload in watched.watcher.poll() {
                let material_index = watched.material_index;
                let result = match reload {
                    GraphReload::Ready(graph) => renderer
                        .matgraphs
                        .set_material(
                            &renderer.device,
                            &renderer.queue,
                            material_index,
                            &graph,
                            &watched.params,
                        )
                        .map(|_| ()),
                    GraphReload::Failed(errors) => Err(MaterialGraphError::Graph(errors)),
                };
                if let Err(error) = result {
                    log::warn!("material {material_index}: graph reload failed: {error}");
                    failures.push(GraphReloadFailure {
                        material_index,
                        error,
                    });
                }
            }
        }
        failures
    }
}

#[cfg(test)]
mod disk_cache_tests {
    use super::*;

    const SOURCE: u64 = 0x1234_5678_9abc_def0;
    const BODY: &str = "fn material_surface_graph_variant() {}\n";

    #[test]
    fn a_cache_file_is_read_back_when_source_and_version_match() {
        let text = cache_text(SOURCE, BODY);
        assert_eq!(read_cached_body(&text, SOURCE), Some(BODY));
    }

    #[test]
    fn a_cache_file_of_another_graph_source_is_rejected() {
        let text = cache_text(SOURCE, BODY);
        assert_eq!(read_cached_body(&text, SOURCE + 1), None);
    }

    #[test]
    fn a_cache_file_with_an_edited_body_is_rejected() {
        let text = cache_text(SOURCE, BODY).replace("{}", "{ }");
        assert_eq!(read_cached_body(&text, SOURCE), None);
    }

    #[test]
    fn a_cache_file_of_another_version_is_rejected() {
        let old = format!(
            "// matgraph-cache v{} format={} source={SOURCE:016x} body={:016x}\n{BODY}",
            DISK_CACHE_VERSION + 1,
            redox_matgraph::FORMAT_VERSION,
            fnv1a(BODY.as_bytes()),
        );
        assert_eq!(read_cached_body(&old, SOURCE), None);
    }

    #[test]
    fn a_file_without_a_header_is_rejected() {
        assert_eq!(read_cached_body(BODY, SOURCE), None);
    }
}
