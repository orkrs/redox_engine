//! Validation and port type inference for [`MaterialGraph`] (MAT-02 step 1).

use std::collections::{HashMap, HashSet};

use crate::graph::{
    FORMAT_VERSION, GraphError, MaterialGraph, Node, NodeId, NodeKind, Ty, arity, input_types,
    output_type,
};

/// Output type of every node of a valid graph.
pub type Types = HashMap<NodeId, Ty>;

impl MaterialGraph {
    /// Checks ids, links, types, cycles, parameters and the surface output.
    /// Returns an empty vector for a valid graph.
    pub fn validate(&self) -> Vec<GraphError> {
        match node_types(self) {
            Ok(_) => Vec::new(),
            Err(errors) => errors,
        }
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

    /// Parameters, constants, arity and the number of surface outputs.
    fn check_static(&mut self, graph: &MaterialGraph) {
        let mut names: HashSet<&str> = HashSet::new();
        let mut surfaces = Vec::new();
        for node in &graph.nodes {
            if node.inputs.len() > arity(&node.kind) {
                self.fail(Some(node.id), "лишние входы");
            }
            match &node.kind {
                NodeKind::Param { name, ty, default } => {
                    self.check_param(node.id, name, *ty, default, &mut names);
                }
                NodeKind::ConstF32(v) if !v.is_finite() => {
                    self.fail(Some(node.id), "константа не конечна");
                }
                NodeKind::ConstVec3(v) if v.iter().any(|x| !x.is_finite()) => {
                    self.fail(Some(node.id), "константа не конечна");
                }
                NodeKind::SurfaceOutput => surfaces.push(node.id),
                _ => {}
            }
        }
        match surfaces.len() {
            0 => self.fail(None, "нет узла SurfaceOutput"),
            1 => {}
            n => self.fail(None, format!("узлов SurfaceOutput {n}, ожидался один")),
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

    /// Checks fixed port types, the arithmetic join rule and vector-only operations.
    fn check_ports(&mut self, node: &Node, actual: &[Ty]) -> Option<Ty> {
        let kind = &node.kind;
        let mut ok = true;
        for (port, (fixed, ty)) in input_types(kind).iter().zip(actual).enumerate() {
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
        if input_types(kind).iter().all(Option::is_none) {
            ok &= self.check_arithmetic(node.id, actual);
        }
        if matches!(kind, NodeKind::Dot | NodeKind::Normalize) {
            for ty in actual {
                if !ty.is_vector() {
                    self.fail(Some(node.id), format!("ожидался вектор, получено {ty:?}"));
                    ok = false;
                }
            }
        }
        ok.then(|| output_type(kind, actual))
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
