use lisp_sitter_core::definers::Definer;
use lisp_sitter_core::treesit_plugin::{DialectSpec, TreesitPlugin};
use lisp_sitter_core::FormInfo;

use crate::treesit::base_definers;

/// Common Lisp dialect specification.
pub struct CommonLispSpec;

impl DialectSpec for CommonLispSpec {
    fn id(&self) -> &'static str {
        "commonlisp"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &[".lisp", ".cl"]
    }

    fn root_kind(&self) -> &'static str {
        "source"
    }

    fn language(&self) -> tree_sitter::Language {
        tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into()
    }

    fn base_definers(&self) -> Vec<Definer> {
        base_definers()
    }

    fn is_known_global(&self, name: &str) -> bool {
        is_cl_global(name)
    }

    fn semantic_check(&self, content: &str, forms: &[FormInfo]) -> Vec<String> {
        let mut warnings = Vec::new();

        // ── check: missing docstrings ───────────────────────────
        for f in forms {
            if f.name.as_deref().is_none() {
                continue;
            }
            let text = &content[f.start..f.end];
            let label = f.label.split(':').next().unwrap_or("");
            let wants_doc = matches!(
                label,
                "defun" | "defmacro" | "defgeneric" | "defmethod" | "defclass"
            );
            if wants_doc && !lisp_sitter_core::has_docstring(text) {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::pos_label(content, f.start, &f.label)
                ));
            }
            let wants_var_doc = matches!(label, "defvar" | "defparameter" | "defconstant");
            if wants_var_doc && !lisp_sitter_core::has_docstring(text) {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::pos_label(content, f.start, &f.label)
                ));
            }
        }

        // ── check: missing (in-package …) ────────────────────────
        let has_in_package = content.contains("(in-package ");
        let defines_something = forms.iter().any(|f| {
            matches!(
                f.label.split(':').next().unwrap_or(""),
                "defun"
                    | "defmacro"
                    | "defclass"
                    | "defgeneric"
                    | "defmethod"
                    | "defvar"
                    | "defparameter"
                    | "defstruct"
            )
        });
        if defines_something && !has_in_package {
            warnings.push(format!(
                "{}: file defines symbols but has no (in-package …) form",
                lisp_sitter_core::pos_label(content, 0, "top")
            ));
        }

        warnings
    }
}

/// The Common Lisp plugin: a [`TreesitPlugin`] driven by [`CommonLispSpec`].
pub struct CommonLispPlugin;

impl CommonLispPlugin {
    #[allow(clippy::new_ret_no_self)] // `CommonLispPlugin` is a constructor namespace
    pub fn new() -> TreesitPlugin {
        TreesitPlugin::new(Box::new(CommonLispSpec))
    }

    pub fn with_extra_definers(extra: &[String]) -> TreesitPlugin {
        TreesitPlugin::with_extra_definers(Box::new(CommonLispSpec), extra)
    }
}

/// Curated (non-exhaustive) set of Common Lisp special operators and common
/// standard functions, used by project analysis to suppress unresolved-call
/// warnings.
fn is_cl_global(name: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let set = SET.get_or_init(|| {
        [
            // special operators / macros
            "if",
            "when",
            "unless",
            "cond",
            "case",
            "ccase",
            "ecase",
            "and",
            "or",
            "not",
            "progn",
            "prog1",
            "prog2",
            "let",
            "let*",
            "flet",
            "labels",
            "macrolet",
            "block",
            "return",
            "return-from",
            "lambda",
            "function",
            "quote",
            "setq",
            "setf",
            "psetf",
            "incf",
            "decf",
            "push",
            "pop",
            "dolist",
            "dotimes",
            "do",
            "do*",
            "loop",
            "multiple-value-bind",
            "destructuring-bind",
            "handler-case",
            "handler-bind",
            "unwind-protect",
            "catch",
            "throw",
            "ignore-errors",
            "eval-when",
            "declaim",
            "declare",
            "the",
            "defun",
            "defmacro",
            "defvar",
            "defparameter",
            "defconstant",
            "defclass",
            "defgeneric",
            "defmethod",
            "defstruct",
            "in-package",
            "with-slots",
            "with-accessors",
            "with-open-file",
            "with-output-to-string",
            "apply",
            "funcall",
            "mapcar",
            "mapc",
            "mapcan",
            "reduce",
            "remove-if",
            "remove-if-not",
            "find-if", // list / sequence
            "car",
            "cdr",
            "caar",
            "cadr",
            "cddr",
            "cons",
            "list",
            "list*",
            "append",
            "nth",
            "nthcdr",
            "first",
            "second",
            "third",
            "rest",
            "length",
            "reverse",
            "nreverse",
            "member",
            "assoc",
            "elt",
            "aref",
            "svref",
            "vector",
            "make-array",
            "make-list",
            "last",
            "butlast",
            "subseq",
            "remove",
            "delete",
            "find",
            "position",
            "count",
            "sort",
            "every",
            "some",
            "notany", // predicates / equality
            "eq",
            "eql",
            "equal",
            "equalp",
            "null",
            "atom",
            "consp",
            "listp",
            "stringp",
            "numberp",
            "integerp",
            "symbolp",
            "functionp",
            "boundp",
            "fboundp",
            "zerop",
            "plusp",
            "minusp",
            "typep",
            "values", // arithmetic / strings
            "+",
            "-",
            "*",
            "/",
            "mod",
            "rem",
            "1+",
            "1-",
            "max",
            "min",
            "abs",
            "expt",
            "sqrt",
            "floor",
            "ceiling",
            "round",
            "truncate",
            "=",
            "/=",
            "<",
            ">",
            "<=",
            ">=",
            "concatenate",
            "format",
            "string",
            "string=",
            "string<",
            "char",
            "substring",
            "parse-integer",
            "write-to-string",
            "symbol-name",
            "intern",
            "make-symbol",
            "gensym",
            // io / hash
            "print",
            "princ",
            "prin1",
            "write",
            "write-line",
            "write-string",
            "error",
            "warn",
            "make-hash-table",
            "gethash",
            "remhash",
            "maphash",
            "getf",
            "get",
        ]
        .into_iter()
        .collect()
    });
    set.contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lisp_sitter_core::edit::{insert_after, replace_node};
    use lisp_sitter_core::LanguagePlugin;

    #[test]
    fn check_valid_file() {
        let content = "(defun foo ()\n  (+ 1 2))\n";
        assert!(CommonLispPlugin::new().check_file(content).is_ok());
    }

    #[test]
    fn replace_defun() {
        let content = "(defun old-f ()\n  1)\n";
        let new_body = "(defun old-f ()\n  2)\n";
        let updated = replace_node(&CommonLispPlugin::new(), content, "old-f", new_body).unwrap();
        assert!(updated.contains("2)"));
        assert!(CommonLispPlugin::new().check_file(&updated).is_ok());
    }

    #[test]
    fn insert_after_form() {
        let content = "(defun first () 1)\n";
        let form = "(defun second () 2)";
        let updated = insert_after(&CommonLispPlugin::new(), content, "first", form).unwrap();
        assert!(updated.contains("defun second"));
    }

    #[test]
    fn outline_labels() {
        let content = "(defun a () 1)\n(defmacro b () '(+ 1 2))\n";
        let tree = CommonLispPlugin::new().outline(content).unwrap();
        assert!(tree.contains("defun:a"));
        assert!(tree.contains("defmacro:b"));
    }

    #[test]
    fn bounds_by_name() {
        let content = "(defun alpha () 1)\n(defun beta () 2)\n";
        let bounds = CommonLispPlugin::new()
            .node_bounds(content, "beta")
            .unwrap();
        assert!(bounds.0 < bounds.1);
    }
}
