use tree_sitter::{Parser, Tree};

use lisp_sitter_core::definers::Definer;
use lisp_sitter_core::sexp_reader::Dialect;
use lisp_sitter_core::treesit_util::{bounds_in_forms, fallback_forms, forms_from_tree};
use lisp_sitter_core::{DefinerSet, FormInfo};

const ROOT_KIND: &str = "source_file";
pub const DIALECT: Dialect = Dialect::Elisp;

/// Base top-level definition forms recognized for Emacs Lisp (before any
/// user-configured `extra_definers` are appended).
pub fn base_definers() -> Vec<Definer> {
    [
        "defun",
        "defsubst",
        "defmacro",
        "cl-defun",
        "cl-defmacro",
        "cl-defsubst",
        "cl-defmethod",
        "cl-defgeneric",
        "cl-defstruct",
        "defvar",
        "defvar-local",
        "defconst",
        "defcustom",
        "defface",
        "defgroup",
        "define-minor-mode",
        "define-derived-mode",
        "define-globalized-minor-mode",
        "define-error",
        "defalias",
        "ert-deftest",
        "define-advice",
    ]
    .into_iter()
    .map(Definer::second)
    .collect()
}

pub fn parse(content: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_elisp::LANGUAGE.into())
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

    fn set() -> DefinerSet {
        DefinerSet::new(base_definers())
    }

    #[test]
    fn parses_defun_outline() {
        let content = "(defun a () 1)\n(defvar b 2)\n";
        let forms = top_level_forms(content, &set());
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "defun:a");
        assert_eq!(forms[1].label, "defvar:b");
    }

    #[test]
    fn recognizes_broad_forms() {
        let content = "(cl-defun f () 1)\n(defcustom o t \"doc\")\n\
                       (define-minor-mode m \"doc\")\n(cl-defmethod g ((x integer)) x)\n\
                       (ert-deftest tst () (should t))\n(defalias 'al 'f)\n";
        let forms = top_level_forms(content, &set());
        let labels: Vec<&str> = forms.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"cl-defun:f"), "{labels:?}");
        assert!(labels.contains(&"defcustom:o"), "{labels:?}");
        assert!(labels.contains(&"define-minor-mode:m"), "{labels:?}");
        assert!(labels.contains(&"cl-defmethod:g"), "{labels:?}");
        assert!(labels.contains(&"ert-deftest:tst"), "{labels:?}");
        assert!(labels.contains(&"defalias:al"), "{labels:?}");
    }

    #[test]
    fn extra_definer_recognized() {
        let mut s = set();
        s.extend_keywords(&["define-widget".to_string()]);
        let forms = top_level_forms("(define-widget my-w 'item \"doc\")\n", &s);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].label, "define-widget:my-w");
    }

    #[test]
    fn bounds_for_minor_mode() {
        let content = "(defun a () 1)\n(define-minor-mode my-mode \"doc\")\n";
        let b = node_bounds(content, &set(), "my-mode").unwrap();
        assert_eq!(&content[b.0..b.1], "(define-minor-mode my-mode \"doc\")");
    }
}
