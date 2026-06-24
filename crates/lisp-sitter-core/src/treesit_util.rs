use tree_sitter::Node;

use crate::definers::DefinerSet;
use crate::sexp_reader::Dialect;
use crate::FormInfo;

pub fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .to_string()
}

/// One [`FormInfo`] per recognized top-level definition under `root`.
///
/// Tree-sitter supplies only the byte boundaries of each top-level child; the
/// head keyword and name are read from the child's source text via `set`, so
/// this is identical whether the grammar emits a dedicated def node or a plain
/// list. Comments and non-definer forms are skipped.
pub fn forms_from_tree(content: &str, root: Node, set: &DefinerSet) -> Vec<FormInfo> {
    let mut forms = Vec::new();
    for child in child_nodes(root) {
        if child.kind() == "comment" {
            continue;
        }
        let (start, end) = (child.start_byte(), child.end_byte());
        if let Some((head, name)) = set.classify(&content[start..end]) {
            forms.push(FormInfo {
                name: Some(name.clone()),
                label: format!("{head}:{name}"),
                start,
                end,
            });
        }
    }
    forms
}

/// The s-expression fallback: recognized definitions found by byte scanning,
/// using the same [`DefinerSet`] as the tree path.
pub fn fallback_forms(content: &str, set: &DefinerSet, dialect: Dialect) -> Vec<FormInfo> {
    crate::sexp_scan::top_level_definer_forms(content, set, dialect)
        .unwrap_or_default()
        .into_iter()
        .map(|f| FormInfo {
            name: Some(f.name.clone()),
            label: format!("{}:{}", f.head, f.name),
            start: f.start,
            end: f.end,
        })
        .collect()
}

/// Byte range of the first form whose name matches `target`.
pub fn bounds_in_forms(forms: &[FormInfo], target: &str) -> Option<(usize, usize)> {
    forms
        .iter()
        .find(|f| f.name.as_deref() == Some(target))
        .map(|f| (f.start, f.end))
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

/// Extract a human‑readable label for a form node. Prefers the form's head
/// keyword text (e.g. `defun`) over the grammar node kind so dedicated nodes
/// and plain lists label consistently.
fn form_label(content: &str, node: tree_sitter::Node) -> Option<String> {
    let kids = meaningful_children(node);

    // Head = first meaningful child's text (the keyword: defun, require, …).
    let head = kids
        .first()
        .map(|n| node_text(*n, content).trim().to_string())
        .filter(|s| !s.is_empty());

    // Name = the `name` field if present, else a second symbol child.
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, content).trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            kids.get(1)
                .filter(|n| matches!(n.kind(), "sym_lit" | "symbol"))
                .map(|n| node_text(*n, content).trim().to_string())
                .filter(|s| !s.is_empty())
        });

    match (head, name) {
        (Some(h), Some(n)) => Some(format!("{h}:{n}")),
        (Some(h), None) => Some(h),
        (None, _) => Some(node.kind().to_string()),
    }
}

/// Check if a form has a docstring — a string literal right after the arglist
/// (for defun/define/defclass) or as the third argument (for defvar/defcustom).
///
/// The scanner watches for:
/// - A `)` at depth 2 → that closes the arglist → `found_after_args = true`.
/// - A `)` at depth 1 on a non‑empty inner list → also marks it.
/// - Then the next `"` signals the docstring.
pub fn has_docstring(form_text: &str) -> bool {
    let mut depth = 0i32;
    let mut found_after_args = false;
    let bytes = form_text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => { depth += 1; i += 1; }
            b')' => {
                // depth==2: closing the arglist (defun f (x) "…")
                // The outer `)` of the defun is at depth==1 and is ignored.
                if depth == 2 {
                    found_after_args = true;
                }
                depth -= 1;
                i += 1;
            }
            b'"' if found_after_args => {
                // Found a string literal right after the arglist — that's the docstring.
                return true;
            }
            b';' => { while i < bytes.len() && bytes[i] != b'\n' { i += 1; } }
            b'#' if i + 1 < bytes.len() && bytes[i+1] == b'|' => {
                while i + 1 < bytes.len() && !(bytes[i] == b'|' && bytes[i+1] == b'#') { i += 1; }
                if i < bytes.len() { i += 2; }
            }
            _ => { i += 1; }
        }
        if depth < 0 { break; }
    }
    false
}

pub fn pos_label(content: &str, start: usize, label: &str) -> String {
    crate::position::pos_label(content, start, label)
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
