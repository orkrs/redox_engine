//! Custom WGSL node (MAT-03 step 3): body checks and the generated function.
//!
//! A custom node is a pure function `fn custom_<node>(inputs) -> output { body }`. The body
//! may not declare bindings, attributes, globals or `enable` directives, and its braces must
//! stay balanced, so it cannot close the generated function early. The function is parsed and
//! validated by naga on its own; errors are reported with the line number inside `body`.

use crate::graph::{GraphError, NodeId, Ty};

/// Largest accepted body, in bytes.
pub const MAX_BODY_BYTES: usize = 4096;

/// WGSL header line of the generated function.
pub fn header(node: NodeId, inputs: &[(String, Ty)], output: Ty) -> String {
    let params: Vec<String> = inputs
        .iter()
        .map(|(name, ty)| format!("{name}: {}", ty.wgsl()))
        .collect();
    format!(
        "fn custom_{node}({}) -> {} {{",
        params.join(", "),
        output.wgsl()
    )
}

/// Full text of the generated function (header, body, closing brace).
pub fn function_text(node: NodeId, inputs: &[(String, Ty)], output: Ty, body: &str) -> String {
    format!("{}\n{body}\n}}\n", header(node, inputs, output))
}

/// Checks a custom node; returns every problem found (empty when valid).
pub fn check(
    node: NodeId,
    inputs: &[(String, Ty)],
    output: Ty,
    body: &str,
) -> Vec<GraphError> {
    let mut errors = Vec::new();
    for (name, ty) in inputs {
        if !is_identifier(name) {
            errors.push(GraphError::new(
                Some(node),
                format!("имя входа «{name}» должно быть идентификатором"),
            ));
        }
        if !is_arithmetic(*ty) {
            errors.push(GraphError::new(
                Some(node),
                format!("вход «{name}» имеет тип {ty:?}, ожидался арифметический"),
            ));
        }
    }
    if !is_arithmetic(output) {
        errors.push(GraphError::new(
            Some(node),
            format!("выход имеет тип {output:?}, ожидался арифметический"),
        ));
    }
    if body.len() > MAX_BODY_BYTES {
        errors.push(GraphError::new(
            Some(node),
            format!(
                "тело custom-узла {} байт, предел {MAX_BODY_BYTES}",
                body.len()
            ),
        ));
    }
    if let Some(problem) = forbidden(body) {
        errors.push(GraphError::new(Some(node), problem));
    }
    if errors.is_empty() {
        errors.extend(parse_and_validate(node, inputs, output, body));
    }
    errors
}

fn is_arithmetic(ty: Ty) -> bool {
    matches!(ty, Ty::F32 | Ty::Vec2 | Ty::Vec3 | Ty::Vec4)
}

/// Syntax rules checked on the text: no attributes, no `var<` globals, no `enable`, and braces
/// must not go negative (which would end the function early).
fn forbidden(body: &str) -> Option<String> {
    if body.contains('@') {
        return Some("атрибуты (@group, @binding и др.) в теле запрещены".to_string());
    }
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("var<") {
        return Some("объявления var<…> в теле запрещены".to_string());
    }
    if tokens(body).any(|t| t == "enable") {
        return Some("директива enable в теле запрещена".to_string());
    }
    let mut depth: i32 = 0;
    for c in body.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return Some("тело выходит за пределы функции (лишняя «}»)".to_string());
        }
    }
    if depth != 0 {
        return Some("тело содержит незакрытые «{»".to_string());
    }
    None
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Parses and validates the generated function with naga. Line numbers in the messages count
/// from the first line of `body`.
fn parse_and_validate(
    node: NodeId,
    inputs: &[(String, Ty)],
    output: Ty,
    body: &str,
) -> Vec<GraphError> {
    let source = function_text(node, inputs, output, body);
    let module = match naga::front::wgsl::parse_str(&source) {
        Ok(module) => module,
        Err(e) => {
            let line = e.location(&source).map(|l| l.line_number);
            return vec![located(node, line, &e.message().to_string())];
        }
    };
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    match validator.validate(&module) {
        Ok(_) => Vec::new(),
        Err(e) => {
            let line = e.location(&source).map(|l| l.line_number);
            vec![located(node, line, &e.to_string())]
        }
    }
}

/// Error for a module line; module line 1 is the header, line `k >= 2` is body line `k - 1`.
fn located(node: NodeId, module_line: Option<u32>, detail: &str) -> GraphError {
    let message = match module_line {
        Some(1) => format!("ошибка в заголовке функции: {detail}"),
        Some(line) => format!("строка {} тела: {detail}", line.saturating_sub(1)),
        None => detail.to_string(),
    };
    GraphError::new(Some(node), message)
}
