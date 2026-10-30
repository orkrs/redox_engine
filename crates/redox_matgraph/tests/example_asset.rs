//! The example `.matgraph` asset loads, validates and compiles (MAT-02 step 6).

#![allow(clippy::unwrap_used)]

use redox_matgraph::{CodegenOptions, Features, MaterialGraph, Pass, compile_variant};

const EXAMPLE: &str = include_str!("../../../assets/materials/example_tint.matgraph");

#[test]
fn example_tint_loads_validates_and_compiles() {
    let graph = MaterialGraph::from_ron(EXAMPLE).unwrap();
    assert!(graph.validate().is_empty());
    let options = CodegenOptions {
        pass: Pass::Main,
        features: Features::default(),
    };
    let variant = compile_variant(&graph, &options).unwrap();
    assert_eq!(variant.ir.params.len(), 1);
    assert_eq!(variant.ir.params[0].name, "tint");
}
