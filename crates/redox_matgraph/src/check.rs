//! Validation and port type inference for [`MaterialGraph`] (MAT-02 step 1, extended by MAT-03).

use std::collections::{HashMap, HashSet};

use crate::custom;
use crate::graph::{
    FORMAT_VERSION, GRAPH_TEXTURE_SLOTS, GraphError, MaterialGraph, Node, NodeId, NodeKind, Ty,
    arity, input_types, output_type,
};

/// Output type of every node of a valid graph.
pub type Types = HashMap<NodeId, Ty>;

/// Texture samples per graph above which a warning is raised (not an error).
pub const SAMPLE_WARNING_LIMIT: usize = 16;

/// Non-fatal observation about a graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphWarning {
    /// Node the warning belongs to, if any.
    pub node: Option<NodeId>,
    /// Message.
    pub message: String,
}

impl MaterialGraph {
    /// Checks ids, links, types, cycles, parameters, custom bodies and the surface output.
    /// Returns an empty vector for a valid graph.
    pub fn validate(&self) -> Vec<GraphError> {
        match node_types(self) {
            Ok(_) => vertex_stage_errors(self),
            Err(errors) => errors,
        }
    }

    /// Non-fatal warnings (texture sample budget).
    pub fn warnings(&self) -> Vec<GraphWarning> {
        let samples: usize = self.nodes.iter().map(|n| texture_samples(&n.kind)).sum();
        if samples > SAMPLE_WARNING_LIMIT {
            vec![GraphWarning {
                node: None,
                message: format!(
                    "текстурных выборок {samples}, рекомендуемый предел {SAMPLE_WARNING_LIMIT}"
                ),
            }]
        } else {
            Vec::new()
        }
    }
}

/// Texture fetches a node performs (a triplanar sample reads three times).
fn texture_samples(kind: &NodeKind) -> usize {
    match kind {
        NodeKind::Texture2d { .. }
        | NodeKind::TextureSample { .. }
        | NodeKind::NormalMap { .. } => 1,
        NodeKind::Triplanar { .. } => 3,
        _ => 0,
    }
}

/// Checks `graph` and returns the output type of every node, or all errors found.
pub fn node_types(graph: &MaterialGraph) -> Result<Types, Vec<GraphError>> {
    if graph.version != FORMAT_VERSION {
        return Err(vec![GraphError::new(
            None,
            format!(
                "версия формата {} не поддерживается, ожидалась {FORMAT_VERSION}",
                graph.version
            ),
        )]);
    }
    let mut checker = Checker::new(graph);
    checker.check_static(graph);
    checker.check_all();
    if !checker.errors.is_empty() {
        return Err(checker.errors);
    }
    Ok(checker
        .memo
        .into_iter()
        .filter_map(|(id, ty)| ty.map(|ty| (id, ty)))
        .collect())
}

/// Walks the graph with memoisation; collects errors instead of stopping at the first.
struct Checker<'a> {
    index: HashMap<NodeId, &'a Node>,
    memo: HashMap<NodeId, Option<Ty>>,
    stack: Vec<NodeId>,
    errors: Vec<GraphError>,
}

impl<'a> Checker<'a> {
    fn new(graph: &'a MaterialGraph) -> Self {
        let mut index = HashMap::new();
        let mut errors = Vec::new();
        for node in &graph.nodes {
            if index.insert(node.id, node).is_some() {
                errors.push(GraphError::new(Some(node.id), "повторяющийся id"));
            }
        }
        Self {
            index,
            memo: HashMap::new(),
            stack: Vec::new(),
            errors,
        }
    }

    fn fail(&mut self, node: Option<NodeId>, detail: impl Into<String>) {
        self.errors.push(GraphError::new(node, detail));
    }

    /// Parameters, constants, custom nodes, arity and the number of surface outputs.
    fn check_static(&mut self, graph: &MaterialGraph) {
        let mut names: HashSet<&str> = HashSet::new();
        let mut surfaces = Vec::new();
        for node in &graph.nodes {
            if node.inputs.len() > arity(&node.kind) {
                self.fail(Some(node.id), "лишние входы");
            }
            self.check_kind_values(node, &mut names);
            self.check_texture_slot(node);
            if matches!(node.kind, NodeKind::SurfaceOutput) {
                surfaces.push(node.id);
            }
        }
        match surfaces.len() {
            0 => self.fail(None, "нет узла SurfaceOutput"),
            1 => {}
            n => self.fail(None, format!("узлов SurfaceOutput {n}, ожидался один")),
        }
    }

    /// A texture node must address one of the bound texture slots.
    fn check_texture_slot(&mut self, node: &Node) {
        let slot = match node.kind {
            NodeKind::Texture2d { slot }
            | NodeKind::TextureSample { slot, .. }
            | NodeKind::NormalMap { slot }
            | NodeKind::Triplanar { slot, .. } => slot,
            _ => return,
        };
        if slot >= GRAPH_TEXTURE_SLOTS {
            self.fail(
                Some(node.id),
                format!("слот текстуры {slot} вне 0..{GRAPH_TEXTURE_SLOTS}"),
            );
        }
    }

    /// Constant and parameter values of one node, and custom bodies.
    fn check_kind_values<'n>(&mut self, node: &'n Node, names: &mut HashSet<&'n str>) {
        let id = node.id;
        match &node.kind {
            NodeKind::Param { name, ty, default } => {
                self.check_param(id, name, *ty, default, names);
            }
            NodeKind::ConstF32(v) if !v.is_finite() => {
                self.fail(Some(id), "константа не конечна");
            }
            NodeKind::ConstVec3(v) if v.iter().any(|x| !x.is_finite()) => {
                self.fail(Some(id), "константа не конечна");
            }
            NodeKind::Remap {
                in_min,
                in_max,
                out_min,
                out_max,
            } => {
                if ![in_min, in_max, out_min, out_max]
                    .iter()
                    .all(|v| v.is_finite())
                {
                    self.fail(Some(id), "параметры remap не конечны");
                } else if in_min == in_max {
                    self.fail(
                        Some(id),
                        "remap: входной диапазон вырожден (in_min = in_max)",
                    );
                }
            }
            NodeKind::Fresnel { power }
            | NodeKind::Triplanar { scale: power, .. }
            | NodeKind::HeightBlend { sharpness: power }
            | NodeKind::NormalFromHeight { strength: power }
            | NodeKind::PositionOffset { max_offset: power }
                if !power.is_finite() =>
            {
                self.fail(Some(id), "число не конечно");
            }
            NodeKind::Split { component } if *component > 3 => {
                self.fail(Some(id), "компонента вне 0..=3");
            }
            NodeKind::Custom {
                inputs,
                output,
                body,
                ..
            } => {
                let errors = custom::check(id, inputs, *output, body);
                self.errors.extend(errors);
            }
            _ => {}
        }
    }

    fn check_param<'n>(
        &mut self,
        id: NodeId,
        name: &'n str,
        ty: Ty,
        default: &[f32; 4],
        names: &mut HashSet<&'n str>,
    ) {
        if !is_identifier(name) {
            self.fail(
                Some(id),
                format!("имя параметра «{name}» должно быть идентификатором"),
            );
        } else if !names.insert(name) {
            self.fail(Some(id), format!("параметр «{name}» объявлен дважды"));
        }
        if !(ty == Ty::F32 || ty.is_vector()) {
            self.fail(
                Some(id),
                format!("параметр имеет тип {ty:?}, ожидался арифметический"),
            );
        }
        if default.iter().any(|v| !v.is_finite()) {
            self.fail(Some(id), "значение параметра по умолчанию не конечно");
        }
    }

    fn check_all(&mut self) {
        let mut ids: Vec<NodeId> = self.index.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            self.ty(id);
        }
    }

    /// Output type of node `id`, or `None` when it (or an input) is invalid.
    fn ty(&mut self, id: NodeId) -> Option<Ty> {
        if let Some(known) = self.memo.get(&id) {
            return *known;
        }
        if let Some(pos) = self.stack.iter().position(|&s| s == id) {
            let chain: Vec<String> = self.stack[pos..]
                .iter()
                .chain(std::iter::once(&id))
                .map(|n| n.to_string())
                .collect();
            self.fail(Some(id), format!("цикл: {}", chain.join(" -> ")));
            return None;
        }
        let node = self.index.get(&id).copied()?;
        self.stack.push(id);
        let result = self.compute(node);
        self.stack.pop();
        self.memo.insert(id, result);
        result
    }

    fn compute(&mut self, node: &'a Node) -> Option<Ty> {
        let kind = &node.kind;
        let expected = input_types(kind);
        let mut actual = Vec::with_capacity(expected.len());
        let mut complete = true;
        for (port, fixed) in expected.iter().enumerate() {
            let linked = node.inputs.get(port).cloned().flatten();
            let ty = match (linked, fixed) {
                (Some(link), _) => self.link_type(node.id, port, link.node, link.port),
                (None, Some(ty)) if matches!(kind, NodeKind::SurfaceOutput) => Some(*ty),
                (None, _) => {
                    self.fail(Some(node.id), format!("вход {port} не подключён"));
                    None
                }
            };
            match ty {
                Some(ty) => actual.push(ty),
                None => complete = false,
            }
        }
        if !complete {
            return None;
        }
        self.check_ports(node, &actual)
    }

    fn link_type(&mut self, node: NodeId, port: usize, source: NodeId, out: u8) -> Option<Ty> {
        if !self.index.contains_key(&source) {
            self.fail(
                Some(node),
                format!("вход {port} ссылается на несуществующий узел {source}"),
            );
            return None;
        }
        if out != 0 {
            self.fail(Some(node), format!("узел {source} не имеет выхода {out}"));
            return None;
        }
        self.ty(source)
    }

    /// Checks fixed port types, the arithmetic join rule and per-kind rules.
    fn check_ports(&mut self, node: &Node, actual: &[Ty]) -> Option<Ty> {
        let kind = &node.kind;
        let fixed_types = input_types(kind);
        let mut ok = true;
        for (port, (fixed, ty)) in fixed_types.iter().zip(actual).enumerate() {
            if let Some(expected) = fixed
                && expected != ty
            {
                self.fail(
                    Some(node.id),
                    format!("вход {port}: ожидалось {expected:?}, получено {ty:?}"),
                );
                ok = false;
            }
        }
        let arithmetic: Vec<Ty> = fixed_types
            .iter()
            .zip(actual)
            .filter(|(fixed, _)| fixed.is_none())
            .map(|(_, ty)| *ty)
            .collect();
        if !arithmetic.is_empty() {
            ok &= self.check_arithmetic(node.id, &arithmetic);
        }
        ok &= self.check_kind_rules(node, actual);
        ok.then(|| output_type(kind, actual))
    }

    /// Rules that depend on the kind: vector-only operands and noise dimensions.
    fn check_kind_rules(&mut self, node: &Node, actual: &[Ty]) -> bool {
        let id = node.id;
        match &node.kind {
            NodeKind::Dot | NodeKind::Normalize => self.require_vectors(id, actual),
            NodeKind::Split { component } => {
                let dim = actual.first().map_or(0, |t| t.components());
                if dim == 0 || usize::from(*component) >= dim {
                    self.fail(
                        Some(id),
                        format!("компонента {component} у значения типа {:?}", actual[0]),
                    );
                    false
                } else {
                    true
                }
            }
            NodeKind::ValueNoise | NodeKind::GradientNoise | NodeKind::Worley => {
                let ok = matches!(actual.first(), Some(Ty::Vec2 | Ty::Vec3));
                if !ok {
                    self.fail(Some(id), "шум принимает Vec2 или Vec3");
                }
                ok
            }
            _ => true,
        }
    }

    fn require_vectors(&mut self, id: NodeId, actual: &[Ty]) -> bool {
        let mut ok = true;
        for ty in actual {
            if !ty.is_vector() {
                self.fail(Some(id), format!("ожидался вектор, получено {ty:?}"));
                ok = false;
            }
        }
        ok
    }

    /// Arithmetic operands: no resources, and all non-scalar operands share one type.
    fn check_arithmetic(&mut self, id: NodeId, actual: &[Ty]) -> bool {
        let mut ok = true;
        for ty in actual {
            if matches!(ty, Ty::Texture2d | Ty::Sampler) {
                self.fail(
                    Some(id),
                    format!("ресурс {ty:?} нельзя использовать в арифметике"),
                );
                ok = false;
            }
        }
        let mut shape: Option<Ty> = None;
        for ty in actual.iter().copied().filter(|t| *t != Ty::F32) {
            match shape {
                None => shape = Some(ty),
                Some(first) if first != ty => {
                    self.fail(
                        Some(id),
                        format!("несовместимые типы входов: ожидалось {first:?}, получено {ty:?}"),
                    );
                    return false;
                }
                Some(_) => {}
            }
        }
        ok
    }
}

/// Whether `name` is a WGSL-safe identifier made of lowercase ASCII, digits and `_`.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Nodes that cannot run in the vertex stage: texture reads and anything using derivatives.
fn fragment_only(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Texture2d { .. }
            | NodeKind::TextureSample { .. }
            | NodeKind::NormalMap { .. }
            | NodeKind::Triplanar { .. }
            | NodeKind::NormalFromHeight { .. }
    )
}

/// Errors of the `PositionOffset` sink: at most one, and no fragment-only node feeding it.
fn vertex_stage_errors(graph: &MaterialGraph) -> Vec<GraphError> {
    let sinks: Vec<&Node> = graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::PositionOffset { .. }))
        .collect();
    let mut errors = Vec::new();
    if sinks.len() > 1 {
        errors.push(GraphError::new(
            None,
            format!(
                "узлов PositionOffset {}, ожидался не больше одного",
                sinks.len()
            ),
        ));
    }
    let Some(sink) = sinks.first() else {
        return errors;
    };
    let index: std::collections::HashMap<NodeId, &Node> =
        graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut stack: Vec<NodeId> = sink.inputs.iter().flatten().map(|l| l.node).collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = index.get(&id) else {
            continue;
        };
        if fragment_only(&node.kind) {
            errors.push(GraphError::new(
                Some(id),
                "узел работает только во фрагментной стадии и не может питать PositionOffset",
            ));
        }
        stack.extend(node.inputs.iter().flatten().map(|l| l.node));
    }
    errors
}
