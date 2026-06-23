use tree_sitter::{Node, Parser, Tree};

use lisp_sitter_core::treesit_util::{child_nodes, meaningful_children, node_text};
use lisp_sitter_core::FormInfo;

pub const DEFINERS: &[&str] = &[
    "defun", "defmacro", "defclass", "defgeneric", "defmethod",
];

pub fn parse(content: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into())
        .ok()?;
    parser.parse(content, None)
}

pub fn top_level_forms(content: &str) -> Vec<FormInfo> {
    match forms_from_tree(content) {
        Some(forms) if !forms.is_empty() => forms,
        _ => sexp_fallback(content),
    }
}

fn forms_from_tree(content: &str) -> Option<Vec<FormInfo>> {
    let tree = parse(content)?;
    let root = tree.root_node();
    if root.kind() != "source" {
        return None;
    }
    let mut forms = Vec::new();
    for child in child_nodes(root) {
        if let Some(info) = form_from_node(child, content) {
            forms.push(info);
        }
    }
    Some(forms)
}

pub fn node_bounds(content: &str, symbol: &str) -> Option<(usize, usize)> {
    let target = symbol.trim();
    match forms_from_tree(content) {
        Some(forms) => bounds_in_forms(&forms, target),
        None => lisp_sitter_core::sexp_scan::find_form_bounds(content, target, DEFINERS),
    }
}

fn bounds_in_forms(forms: &[FormInfo], target: &str) -> Option<(usize, usize)> {
    for form in forms {
        let name = form.name.as_deref()?;
        if name != target || !replaceable_label(&form.label) {
            continue;
        }
        return Some((form.start, form.end));
    }
    None
}

fn sexp_fallback(content: &str) -> Vec<FormInfo> {
    lisp_sitter_core::sexp_scan::top_level_definer_forms(content, DEFINERS)
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

fn form_from_node(node: Node, source: &str) -> Option<FormInfo> {
    match node.kind() {
        "list_lit" => form_from_list_lit(node, source),
        "defun" | "defmacro" | "defclass" | "defgeneric" | "defmethod" => {
            form_from_def_node(node, source)
        }
        _ => None,
    }
}

fn form_from_list_lit(node: Node, source: &str) -> Option<FormInfo> {
    for inner in child_nodes(node) {
        if matches!(
            inner.kind(),
            "defun" | "defmacro" | "defclass" | "defgeneric" | "defmethod"
        ) {
            return form_from_def_node(inner, source);
        }
    }
    name_from_plain_list(node, source).map(|(head, name)| FormInfo {
        name: Some(name.clone()),
        label: format!("{head}:{name}"),
        start: node.start_byte(),
        end: node.end_byte(),
    })
}

fn form_from_def_node(node: Node, source: &str) -> Option<FormInfo> {
    let keyword = def_node_keyword(node, source).unwrap_or_else(|| node.kind().to_string());
    let name = name_from_def_node(node, source)?;
    Some(FormInfo {
        name: Some(name.clone()),
        label: format!("{keyword}:{name}"),
        start: node.start_byte(),
        end: node.end_byte(),
    })
}

fn def_node_keyword(node: Node, source: &str) -> Option<String> {
    for child in child_nodes(node) {
        if child.kind() == "defun_header" || child.kind().ends_with("_header") {
            for inner in meaningful_children(child) {
                if inner.kind().ends_with("_keyword") || inner.kind() == "defun_keyword" {
                    return Some(node_text(inner, source).trim().to_string());
                }
            }
        }
    }
    None
}

fn name_from_def_node(node: Node, source: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = node_text(name_node, source).trim().to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }
    for child in child_nodes(node) {
        if child.kind() == "defun_header" || child.kind().ends_with("_header") {
            if let Some(name) = sym_after_keyword(child, source) {
                return Some(name);
            }
        }
    }
    None
}

fn sym_after_keyword(header: Node, source: &str) -> Option<String> {
    let mut seen_keyword = false;
    for child in meaningful_children(header) {
        if child.kind().ends_with("_keyword") || child.kind() == "defun_keyword" {
            seen_keyword = true;
            continue;
        }
        if seen_keyword && child.kind() == "sym_lit" {
            return Some(node_text(child, source).trim().to_string());
        }
    }
    None
}

fn name_from_plain_list(node: Node, source: &str) -> Option<(String, String)> {
    let kids = meaningful_children(node);
    let head = kids.first().filter(|k| k.kind() == "sym_lit")?;
    let head_text = node_text(*head, source);
    if !DEFINERS.iter().any(|d| head_text == *d) {
        return None;
    }
    let name_node = kids.get(1).filter(|k| k.kind() == "sym_lit")?;
    Some((head_text, node_text(*name_node, source).trim().to_string()))
}

pub fn has_parse_errors(content: &str) -> bool {
    parse(content)
        .map(|tree| tree.root_node().has_error())
        .unwrap_or(true)
}

pub fn replaceable_label(label: &str) -> bool {
    DEFINERS
        .iter()
        .any(|d| label.starts_with(&format!("{d}:")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defun_and_defmacro() {
        let content = "(defun alpha () 1)\n(defmacro beta () '(+ 1 2))\n";
        let forms = top_level_forms(content);
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "defun:alpha");
        assert_eq!(forms[1].label, "defmacro:beta");
    }

    #[test]
    fn parses_defclass_list_lit() {
        let content = "(defclass foo () ((slot :initform 0)))\n";
        let forms = top_level_forms(content);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("foo"));
        assert!(forms[0].label.starts_with("defclass:"));
    }
}
