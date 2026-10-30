//! Material graph files bound to material slots, loaded and hot-reloaded from the app (MAT-02).
//!
//! [`MaterialGraphPlugin`] installs the [`MaterialGraphBindings`] resource and a system on
//! [`Stage::Render`]. A binding is registered on the plugin, or later through the resource. Each
//! frame the system loads the pending bindings into their slots and watches their files; changed
//! files are rebuilt by [`MaterialGraphHotReload`]. Without bindings the system does nothing, so
//! an app that never binds a graph renders exactly as before.

use std::path::PathBuf;

use redox_ecs::World;
use redox_matgraph::load_graph_file;
use redox_render::context::RenderContext;
use redox_render::matgraph_cache::MaterialGraphHotReload;

use crate::app::AppBuilder;
use crate::dispatcher::Stage;
use crate::plugin::Plugin;

/// One `.matgraph` file bound to a material slot.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphBinding {
    /// Material slot the graph is drawn with.
    pub material_index: usize,
    /// File to load and watch.
    pub path: PathBuf,
    /// Parameter values by slot; missing entries use the slot defaults.
    pub params: Vec<[f32; 4]>,
}

/// Bindings not loaded yet, and the hot reload that watches the loaded ones.
#[derive(Default)]
pub struct MaterialGraphBindings {
    pending: Vec<GraphBinding>,
    reload: MaterialGraphHotReload,
}

impl MaterialGraphBindings {
    /// Empty bindings with the default poll interval.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds `path` to material slot `material_index`. It is loaded and watched on the next
    /// frame. A later bind of the same slot replaces the earlier one.
    pub fn bind(&mut self, material_index: usize, path: impl Into<PathBuf>, params: Vec<[f32; 4]>) {
        self.pending.retain(|b| b.material_index != material_index);
        self.pending.push(GraphBinding {
            material_index,
            path: path.into(),
            params,
        });
    }

    /// Bindings waiting for their first load.
    pub fn pending(&self) -> &[GraphBinding] {
        &self.pending
    }
}

/// Material graph hot reload (MAT-02 step 6). Add it to the app and bind files with
/// [`Self::bind`] or later through [`MaterialGraphBindings`].
#[derive(Default)]
pub struct MaterialGraphPlugin {
    bindings: Vec<GraphBinding>,
}

impl MaterialGraphPlugin {
    /// No bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds `path` to material slot `material_index` with default parameters.
    pub fn bind(self, material_index: usize, path: impl Into<PathBuf>) -> Self {
        self.bind_with_params(material_index, path, Vec::new())
    }

    /// Binds `path` to material slot `material_index` with the given parameter values.
    pub fn bind_with_params(
        mut self,
        material_index: usize,
        path: impl Into<PathBuf>,
        params: Vec<[f32; 4]>,
    ) -> Self {
        self.bindings.push(GraphBinding {
            material_index,
            path: path.into(),
            params,
        });
        self
    }
}

impl Plugin for MaterialGraphPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let bindings = self.bindings.clone();
        app.on_setup(move |world| {
            let mut state = MaterialGraphBindings::new();
            for binding in bindings {
                state.bind(binding.material_index, binding.path, binding.params);
            }
            world.insert_resource(state);
        });
        app.add_system_mut(Stage::Render, |world, renderer| {
            sync_material_graphs(world, renderer);
        });
    }
}

/// Loads the pending bindings and polls the watched files. Rebuild failures are logged by the
/// reload itself and the slot keeps its previous graph.
fn sync_material_graphs(world: &mut World, renderer: &mut RenderContext) {
    let Some(bindings) = world.get_resource_mut::<MaterialGraphBindings>() else {
        return;
    };
    for binding in std::mem::take(&mut bindings.pending) {
        // Watch before reading, so a change made during the load is seen by the next poll.
        bindings.reload.watch(
            binding.material_index,
            &binding.path,
            binding.params.clone(),
        );
        match load_graph_file(&binding.path) {
            // set_material_graph logs its own failures and keeps the previous graph.
            Ok(graph) => {
                let _ =
                    renderer.set_material_graph(binding.material_index, &graph, &binding.params);
            }
            Err(errors) => {
                let text: Vec<String> = errors.iter().map(ToString::to_string).collect();
                log::warn!(
                    "material {}: graph file {} not loaded: {}",
                    binding.material_index,
                    binding.path.display(),
                    text.join("; ")
                );
            }
        }
    }
    let _ = bindings.reload.update(renderer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_plugin_binds_nothing() {
        assert!(MaterialGraphPlugin::new().bindings.is_empty());
    }

    #[test]
    fn a_plugin_keeps_its_bindings_in_order() {
        let plugin = MaterialGraphPlugin::new()
            .bind(0, "a.matgraph")
            .bind_with_params(2, "b.matgraph", vec![[1.0, 0.0, 0.0, 0.0]]);
        assert_eq!(plugin.bindings.len(), 2);
        assert_eq!(plugin.bindings[1].material_index, 2);
        assert_eq!(plugin.bindings[1].params, vec![[1.0, 0.0, 0.0, 0.0]]);
    }

    #[test]
    fn a_later_bind_of_a_slot_replaces_the_pending_one() {
        let mut bindings = MaterialGraphBindings::new();
        bindings.bind(3, "first.matgraph", Vec::new());
        bindings.bind(3, "second.matgraph", Vec::new());
        bindings.bind(4, "other.matgraph", Vec::new());
        let slots: Vec<(usize, &str)> = bindings
            .pending()
            .iter()
            .map(|b| (b.material_index, b.path.to_str().unwrap_or_default()))
            .collect();
        assert_eq!(slots, vec![(3, "second.matgraph"), (4, "other.matgraph")]);
    }
}
