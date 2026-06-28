use tree_sitter::{Parser, Tree};

use lisp_sitter_core::definers::{Definer, NameStrategy};
use lisp_sitter_core::sexp_reader::Dialect;
use lisp_sitter_core::treesit_util::{bounds_in_forms, fallback_forms, forms_from_tree};
use lisp_sitter_core::{DefinerSet, FormInfo};

const ROOT_KIND: &str = "program";
pub const DIALECT: Dialect = Dialect::Generic;

/// Base top-level definition forms recognized for Scheme.
pub fn base_definers() -> Vec<Definer> {
    let mut defs: Vec<Definer> = ["define", "define-syntax", "define-record-type", "define-values", "define-structure"]
        .into_iter()
        .map(Definer::second)
        .collect();
    defs.push(Definer::new("define-library", NameStrategy::LibraryList));
    defs
}

pub fn parse(content: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_scheme::LANGUAGE.into())
        .ok()?;
    parser.parse(content, None)
}

pub fn top_level_forms(content: &str, set: &DefinerSet) -> Vec<FormInfo> {
    if let Some(tree) = parse(content) {
        let root = tree.root_node();
        if root.kind() == ROOT_KIND {
            let forms = forms_from_tree(content, root, set);
            if !forms.is_empty() {
                return forms;
            }
        }
    }
    fallback_forms(content, set, DIALECT)
}

pub fn node_bounds(content: &str, set: &DefinerSet, symbol: &str) -> Option<(usize, usize)> {
    bounds_in_forms(&top_level_forms(content, set), symbol.trim())
}

pub fn has_parse_errors(content: &str) -> bool {
    parse(content)
        .map(|tree| tree.root_node().has_error())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_define_forms() {
        let content = "(define foo 1)\n(define (bar x) (+ x 1))\n";
        let forms = top_level_forms(content, &DefinerSet::new(base_definers()));
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "define:foo");
        assert_eq!(forms[1].label, "define:bar");
    }

    #[test]
    fn parses_define_library() {
        let content = "(define-library (my lib)\n  (export foo))\n";
        let forms = top_level_forms(content, &DefinerSet::new(base_definers()));
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("my lib"));
    }

    #[test]
    fn parses_define_record_type() {
        let content = "(define-record-type point (make-point x y) point? (x point-x) (y point-y))\n";
        let forms = top_level_forms(content, &DefinerSet::new(base_definers()));
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("point"));
    }
}
