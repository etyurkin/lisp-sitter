use tree_sitter::Node;

pub fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .to_string()
}

pub fn child_nodes(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

pub fn meaningful_children(node: Node) -> Vec<Node> {
    child_nodes(node)
        .into_iter()
        .filter(|c: &Node| !matches!(c.kind(), "(" | ")"))
        .collect()
}

pub fn validate_treesit(content: &str, has_parse_errors: bool) -> crate::Result<()> {
    if let Some(err) = crate::scan::scan_parens(content) {
        return Err(crate::error::Error::Syntax(err));
    }
    if has_parse_errors {
        return Err(crate::error::Error::Syntax(
            crate::position::error_at(content, 0, "tree-sitter parse error"),
        ));
    }
    Ok(())
}

/// Recursively walk a tree‑sitter node up to `max_depth` and produce an
/// indented outline.  Only nodes that look like "forms" (lists, function
/// definitions, etc.) are printed — leaf tokens are skipped.
pub fn recursive_outline(content: &str, node: tree_sitter::Node, max_depth: usize) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        write_sub_tree(content, child, max_depth, 0, &mut out);
    }
    out
}

fn write_sub_tree(
    content: &str,
    node: tree_sitter::Node,
    max_depth: usize,
    cur_depth: usize,
    out: &mut String,
) {
    if !is_form_like(&node) {
        return;
    }

    // Emit label for this node
    if let Some(label) = form_label(content, node) {
        if !out.is_empty() {
            out.push('\n');
        }
        for _ in 0..cur_depth {
            out.push_str("  ");
        }
        let (line, col) = crate::position::line_column(content, node.start_byte());
        out.push_str(&format!("{label}@{line}:{col}"));
    }

    if cur_depth >= max_depth {
        return;
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        write_sub_tree(content, child, max_depth, cur_depth + 1, out);
    }
}

/// True if a tree‑sitter node represents a "form" we'd show in a tree view:
/// lists, function/macro/class definitions, special forms, vectors.
fn is_form_like(node: &tree_sitter::Node) -> bool {
    matches!(
        node.kind(),
        "list"
            | "list_lit"
            | "function_definition"
            | "macro_definition"
            | "special_form"
            | "defun"
            | "defmacro"
            | "defclass"
            | "defgeneric"
            | "defmethod"
            | "define"
            | "define_syntax"
            | "define_library"
            | "let"
            | "let_binding"
            | "body"
            | "function_body"
            | "parenthesized_list"
            | "vector"
            | "quoted"
    )
}

/// Extract a human‑readable label for a form node.
fn form_label(content: &str, node: tree_sitter::Node) -> Option<String> {
    // 1. If the node has a `name` field, use $kind:$name
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(name_node, content).trim().to_string();
        if !name.is_empty() {
            return Some(format!("{}:{}", node.kind(), name));
        }
    }

    let kids = meaningful_children(node);

    // 2. First meaningful child gives the "head" symbol
    if let Some(first) = kids.first() {
        let head = node_text(*first, content).trim().to_string();
        if !head.is_empty() {
            // If there's a second symbol child, use $head:$name format
            if let Some(second) = kids.get(1) {
                let kind = second.kind();
                if kind == "sym_lit" || kind == "symbol" {
                    let sym = node_text(*second, content).trim().to_string();
                    if !sym.is_empty() {
                        return Some(format!("{head}:{sym}"));
                    }
                }
            }
            return Some(head);
        }
    }

    // 3. Fallback: just the node kind
    Some(node.kind().to_string())
}

pub fn outline_lines(content: &str, forms: &[crate::FormInfo]) -> crate::Result<String> {
    if forms.is_empty() {
        if content.trim().is_empty() {
            return Ok("No forms".to_string());
        }
        return Ok("No forms".to_string());
    }
    Ok(forms
        .iter()
        .map(|f| crate::position::pos_label(content, f.start, &f.label))
        .collect::<Vec<_>>()
        .join("\n"))
}
