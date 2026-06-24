use tree_sitter::{Parser, Tree};

use lisp_sitter_core::definers::Definer;
use lisp_sitter_core::sexp_reader::Dialect;
use lisp_sitter_core::treesit_util::{bounds_in_forms, fallback_forms, forms_from_tree};
use lisp_sitter_core::{DefinerSet, FormInfo};

const ROOT_KIND: &str = "source";
pub const DIALECT: Dialect = Dialect::Generic;

/// Base top-level definition forms recognized for Common Lisp.
pub fn base_definers() -> Vec<Definer> {
    [
        "defun", "defmacro", "defclass", "defgeneric", "defmethod", "defvar",
        "defparameter", "defconstant", "defstruct", "deftype", "define-condition",
        "defpackage", "in-package", "defsetf", "define-compiler-macro",
        "define-symbol-macro",
    ]
    .into_iter()
    .map(Definer::second)
    .collect()
}

pub fn parse(content: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into())
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
    fn parses_defun_and_defmacro() {
        let content = "(defun alpha () 1)\n(defmacro beta () '(+ 1 2))\n";
        let forms = top_level_forms(content, &set());
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "defun:alpha");
        assert_eq!(forms[1].label, "defmacro:beta");
    }

    #[test]
    fn parses_defclass_list_lit() {
        let content = "(defclass foo () ((slot :initform 0)))\n";
        let forms = top_level_forms(content, &set());
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("foo"));
        assert!(forms[0].label.starts_with("defclass:"));
    }

    #[test]
    fn recognizes_broad_forms() {
        let content = "(defvar *x* 1)\n(defparameter *y* 2)\n(defpackage :my-pkg (:use :cl))\n\
                       (in-package :my-pkg)\n(define-condition my-err (error) ())\n(defstruct point x y)\n";
        let forms = top_level_forms(content, &set());
        let labels: Vec<&str> = forms.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"defvar:*x*"), "{labels:?}");
        assert!(labels.contains(&"defparameter:*y*"), "{labels:?}");
        assert!(labels.contains(&"defpackage:my-pkg"), "{labels:?}");
        assert!(labels.contains(&"in-package:my-pkg"), "{labels:?}");
        assert!(labels.contains(&"define-condition:my-err"), "{labels:?}");
        assert!(labels.contains(&"defstruct:point"), "{labels:?}");
    }

    #[test]
    fn defmethod_qualifier_name() {
        let content = "(defmethod foo :around ((x integer)) x)\n";
        let forms = top_level_forms(content, &set());
        assert_eq!(forms[0].name.as_deref(), Some("foo"));
    }
}
