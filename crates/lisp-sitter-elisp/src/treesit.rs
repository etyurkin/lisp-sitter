use tree_sitter::{Node, Parser, Tree};

pub fn parse(content: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_elisp::LANGUAGE.into())
        .ok()?;
    parser.parse(content, None)
}

pub fn top_level_forms(content: &str) -> Vec<lisp_sitter_core::FormInfo> {
    let Some(tree) = parse(content) else {
        return Vec::new();
    };
    let root = tree.root_node();
    if root.kind() != "source_file" {
        return Vec::new();
    }
    let mut forms = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if let Some(info) = form_info_from_node(child, content) {
            forms.push(info);
        }
    }
    forms
}

fn form_info_from_node(node: Node, source: &str) -> Option<lisp_sitter_core::FormInfo> {
    let label = label_from_node(node, source)?;
    let name = name_from_node(node, source);
    Some(lisp_sitter_core::FormInfo {
        name,
        label,
        start: node.start_byte(),
        end: node.end_byte(),
    })
}

fn label_from_node(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_definition" => name_from_node(node, source).map(|n| format!("defun:{n}")),
        "macro_definition" => name_from_node(node, source).map(|n| format!("defmacro:{n}")),
        "special_form" => special_form_label(node, source),
        "list" => list_label(node, source),
        other => Some(other.to_string()),
    }
}

fn name_from_node(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_definition" | "macro_definition" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, source).trim().to_string())
            .filter(|s| !s.is_empty()),
        "list" => {
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            if children.len() < 2 {
                return None;
            }
            let kw = node_text(children[0], source);
            if matches!(kw.as_str(), "defun" | "defsubst" | "defmacro" | "cl-defun") {
                return Some(node_text(children[1], source).trim().to_string());
            }
            None
        }
        "special_form" => special_form_name(node, source),
        _ => None,
    }
}

fn list_label(node: Node, source: &str) -> Option<String> {
    let name = name_from_node(node, source)?;
    let mut cursor = node.walk();
    let first = node.children(&mut cursor).next()?;
    let kw = node_text(first, source);
    if matches!(kw.as_str(), "defun" | "defsubst" | "defmacro" | "cl-defun") {
        Some(format!("{kw}:{name}"))
    } else {
        Some(format!("{kw}:{name}"))
    }
}

fn special_form_name(node: Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    for prefix in ["defvar", "defconst", "defcustom"] {
        if let Some(name) = parse_def_form_name(&text, prefix) {
            return Some(name);
        }
    }
    None
}

fn special_form_label(node: Node, source: &str) -> Option<String> {
    let text = node_text(node, source);
    for prefix in ["defvar", "defconst", "defcustom"] {
        if let Some(name) = parse_def_form_name(&text, prefix) {
            return Some(format!("{prefix}:{name}"));
        }
    }
    None
}

fn parse_def_form_name(text: &str, prefix: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('(') {
        return None;
    }
    let rest = trimmed.strip_prefix('(')?.trim_start();
    if !rest.starts_with(prefix) {
        return None;
    }
    let rest = rest[prefix.len()..].trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ')')
        .unwrap_or(rest.len());
    let name = rest[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .to_string()
}

pub fn has_parse_errors(content: &str) -> bool {
    parse(content)
        .map(|tree| tree.root_node().has_error())
        .unwrap_or(true)
}

pub fn replaceable_label(label: &str) -> bool {
    label.starts_with("defun:")
        || label.starts_with("defmacro:")
        || label.starts_with("defsubst:")
        || label.starts_with("cl-defun:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defun_outline() {
        let content = "(defun a () 1)\n(defvar b 2)\n";
        let forms = top_level_forms(content);
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "defun:a");
        assert_eq!(forms[1].label, "defvar:b");
    }
}
