//! Per-slot [`MaterialInstance`] state and frame sync (MAT-10).

use std::collections::HashMap;

use redox_matgraph::MaterialGraph;

use crate::material::{Material, MaterialGraphId, MaterialInstance};
use crate::matgraph_cache::MaterialGraphError;

impl crate::renderer::Renderer {
    pub fn register_material_graph(&mut self, id: MaterialGraphId, graph: MaterialGraph) {
        self.registered_graphs.insert(id, graph);
    }

    pub fn register_material_graphs_by_id(&mut self, graphs: HashMap<u32, MaterialGraph>) {
        for (id, graph) in graphs {
            self.registered_graphs.insert(MaterialGraphId(id), graph);
        }
    }

    pub fn material_instance(&self, index: usize) -> Option<&MaterialInstance> {
        self.material_instances.get(index)
    }

    pub fn set_material_instance_slot(
        &mut self,
        index: usize,
        instance: MaterialInstance,
        params: Vec<[f32; 4]>,
    ) {
        if index >= self.material_instances.len() {
            self.material_instances
                .resize(index + 1, MaterialInstance::new(Material::default()));
            self.material_graph_params.resize(index + 1, Vec::new());
        }
        self.material_instances[index] = instance;
        self.material_graph_params[index] = params;
    }

    pub fn sync_material_instances(&mut self) {
        let graphs = self.registered_graphs.clone();
        for index in 0..self.material_instances.len() {
            let instance = self.material_instances[index].clone();
            let params = self.material_graph_params[index].clone();
            if let Err(MaterialGraphError::UnknownGraph(id)) =
                self.apply_material_instance(index, &instance, &graphs, &params)
            {
                if self.unknown_graph_logged.insert(id) {
                    log::warn!("material {index}: unknown material graph {}", id.0);
                }
            }
        }
    }

    pub(crate) fn ensure_instance_slots(&mut self, material_index: usize, material: &Material) {
        if material_index >= self.material_instances.len() {
            self.material_instances
                .resize(material_index + 1, MaterialInstance::new(material.clone()));
            self.material_graph_params
                .resize(material_index + 1, Vec::new());
        } else {
            self.material_instances[material_index].material = material.clone();
        }
    }

    pub(crate) fn apply_instance_from_material_data(
        &mut self,
        index: usize,
        graph: Option<MaterialGraphId>,
        params: Vec<[f32; 4]>,
    ) {
        if index < self.material_instances.len() {
            self.material_instances[index].graph = graph;
            self.material_graph_params[index] = params;
        }
    }

    fn apply_material_instance(
        &mut self,
        material_index: usize,
        instance: &MaterialInstance,
        graphs: &HashMap<MaterialGraphId, MaterialGraph>,
        params: &[[f32; 4]],
    ) -> Result<(), MaterialGraphError> {
        let Some(id) = instance.graph else {
            self.matgraphs.clear_material(material_index);
            return Ok(());
        };
        let Some(graph) = graphs.get(&id) else {
            return Err(MaterialGraphError::UnknownGraph(id));
        };
        self.set_material_graph(material_index, graph, params)
    }
}
