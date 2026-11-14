//! Maps graph ids to compiled [`MaterialGraph`] sources (MAT-10 step 1).

use std::collections::HashMap;

use redox_asset::AssetGuid;
use redox_matgraph::MaterialGraph;

pub fn graph_id_for_guid(guid: AssetGuid) -> u32 {
    let g = guid.0;
    (g as u32) ^ ((g >> 32) as u32) ^ ((g >> 64) as u32) ^ ((g >> 96) as u32)
}

#[derive(Clone, Debug, Default)]
pub struct MaterialGraphRegistry {
    graphs: HashMap<u32, MaterialGraph>,
    guid_to_id: HashMap<AssetGuid, u32>,
}

impl MaterialGraphRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, id: u32, graph: MaterialGraph) {
        self.graphs.insert(id, graph);
    }

    pub fn register_guid(&mut self, guid: AssetGuid, graph: MaterialGraph) -> u32 {
        let id = graph_id_for_guid(guid);
        self.guid_to_id.insert(guid, id);
        self.graphs.insert(id, graph);
        id
    }

    pub fn get(&self, id: u32) -> Option<&MaterialGraph> {
        self.graphs.get(&id)
    }

    pub fn as_map(&self) -> HashMap<u32, MaterialGraph> {
        self.graphs.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_graph_is_found_by_id() {
        let mut reg = MaterialGraphRegistry::new();
        let graph = MaterialGraph {
            version: 1,
            name: "t".into(),
            nodes: vec![],
        };
        reg.register(7, graph.clone());
        assert_eq!(reg.get(7), Some(&graph));
    }
}