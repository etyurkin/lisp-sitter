use tree_sitter::{Node, Parser, Tree};

use lisp_sitter_core::treesit_util::{child_nodes, meaningful_children, node_text};
use lisp_sitter_core::FormInfo;

pub const DEFINERS: &[&str] = &["define", "define-syntax", "define-library"];

pub fn parse(content: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_scheme::LANGUAGE.into())
        .ok()?;
    parser.parse(content, None)
}

pub fn top_level_forms_with_library(content: &str) -> Vec<FormInfo> {
    match forms_from_tree(content) {
        Some(forms) if !forms.is_empty() => forms,
        _ => sexp_fallback(content),
    }
}

fn forms_from_tree(content: &str) -> Option<Vec<FormInfo>> {
    let tree = parse(content)?;
    let root = tree.root_node();
    if root.kind() != "program" {
        return None;
    }
    let mut forms = Vec::new();
    for child in child_nodes(root) {
        if child.kind() != "list" {
            continue;
        }
        if let Some((head, name)) = name_from_define_list_with_library(child, content) {
            forms.push(FormInfo {
                name: Some(name.clone()),
                label: format!("{head}:{name}"),
                start: child.start_byte(),
                end: child.end_byte(),
            });
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

fn name_from_define_list(node: Node, source: &str) -> Option<(String, String)> {
    let kids = meaningful_children(node);
    let head_node = kids.first()?;
    if head_node.kind() != "symbol" {
        return None;
    }
    let head = node_text(*head_node, source);
    if !DEFINERS.iter().any(|d| head == *d) {
        return None;
    }
    let name_node = kids.get(1)?;
    let name = match name_node.kind() {
        "symbol" => node_text(*name_node, source),
        "list" => first_symbol_in_list(*name_node, source)?,
        _ => return None,
    };
    if name.is_empty() {
        None
    } else {
        Some((head, name))
    }
}

fn first_symbol_in_list(node: Node, source: &str) -> Option<String> {
    if node.kind() == "symbol" {
        return Some(node_text(node, source));
    }
    for child in meaningful_children(node) {
        if child.kind() == "symbol" {
            return Some(node_text(child, source));
        }
        if child.kind() == "list" {
            if let Some(sym) = first_symbol_in_list(child, source) {
                return Some(sym);
            }
        }
    }
    None
}

fn library_name_from_list(node: Node, source: &str) -> Option<String> {
    let mut parts = Vec::new();
    for child in meaningful_children(node) {
        if child.kind() == "symbol" {
            parts.push(node_text(child, source));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn name_from_define_list_with_library(node: Node, source: &str) -> Option<(String, String)> {
    let kids = meaningful_children(node);
    let head_node = kids.first()?;
    if head_node.kind() != "symbol" {
        return None;
    }
    let head = node_text(*head_node, source);
    if head == "define-library" {
        let lib = kids.get(1)?;
        let name = if lib.kind() == "list" {
            library_name_from_list(*lib, source)?
        } else {
            node_text(*lib, source)
        };
        return Some((head, name));
    }
    name_from_define_list(node, source)
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
    fn parses_define_forms() {
        let content = "(define foo 1)\n(define (bar x) (+ x 1))\n";
        let forms = top_level_forms_with_library(content);
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "define:foo");
        assert_eq!(forms[1].label, "define:bar");
    }

    #[test]
    fn parses_define_library() {
        let content = "(define-library (my lib)\n  (export foo))\n";
        let forms = top_level_forms_with_library(content);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("my lib"));
    }
}
