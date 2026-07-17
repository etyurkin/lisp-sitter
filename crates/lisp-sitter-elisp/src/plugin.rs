use lisp_sitter_core::definers::Definer;
use lisp_sitter_core::treesit_plugin::{DialectSpec, TreesitPlugin};
use lisp_sitter_core::{Dialect, FormInfo};

use crate::treesit::base_definers;

/// Emacs Lisp dialect specification. Structural behavior comes from
/// [`TreesitPlugin`]; this supplies only the elisp-specific pieces.
pub struct ElispSpec;

impl DialectSpec for ElispSpec {
    fn id(&self) -> &'static str {
        "elisp"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &[".el"]
    }

    fn dialect(&self) -> Dialect {
        Dialect::Elisp
    }

    fn root_kind(&self) -> &'static str {
        "source_file"
    }

    fn language(&self) -> tree_sitter::Language {
        tree_sitter_elisp::LANGUAGE.into()
    }

    fn base_definers(&self) -> Vec<Definer> {
        base_definers()
    }

    fn wrap_node(&self, node: &str) -> String {
        // Newline before the closing paren so a trailing line comment inside
        // `node` (e.g. `(foo) ; note`) doesn't swallow it.
        format!("(progn {}\n)", node.trim())
    }

    fn noop_stub(&self) -> &'static str {
        "ignore"
    }

    fn is_known_global(&self, name: &str) -> bool {
        is_elisp_global(name)
    }

    fn semantic_check(&self, content: &str, forms: &[FormInfo]) -> Vec<String> {
        let mut warnings = Vec::new();

        // ── check: missing docstrings ───────────────────────────
        for f in forms {
            if f.name.as_deref().is_none() {
                continue;
            }
            let text = &content[f.start..f.end];
            let head = f.label.split(':').next().unwrap_or("");
            let is_def = matches!(head, "defun" | "defsubst" | "cl-defun" | "defmacro");
            if is_def && !lisp_sitter_core::has_docstring(text) {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::pos_label(content, f.start, &f.label)
                ));
            }
            let is_defvar = matches!(head, "defvar" | "defconst" | "defcustom");
            if is_defvar && !lisp_sitter_core::has_docstring(text) && !text.contains("&define") {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::pos_label(content, f.start, &f.label)
                ));
            }
        }

        // ── check: missing (provide '…) ─────────────────────────
        let has_provide = content.contains("(provide ");
        let defines_something = forms.iter().any(|f| {
            matches!(
                f.label.split(':').next().unwrap_or(""),
                "defun"
                    | "defsubst"
                    | "defmacro"
                    | "cl-defun"
                    | "defvar"
                    | "defconst"
                    | "defcustom"
            )
        });
        if defines_something && !has_provide {
            warnings.push(format!(
                "{}: file defines symbols but has no (provide '…) form",
                lisp_sitter_core::pos_label(content, 0, "top")
            ));
        }

        warnings
    }
}

/// The Emacs Lisp plugin: a [`TreesitPlugin`] driven by [`ElispSpec`].
pub struct ElispPlugin;

impl ElispPlugin {
    /// Plugin with the built-in Emacs Lisp definer set.
    #[allow(clippy::new_ret_no_self)] // `ElispPlugin` is a constructor namespace
    pub fn new() -> TreesitPlugin {
        TreesitPlugin::new(Box::new(ElispSpec))
    }

    /// Plugin whose definer set also recognizes the given extra keywords.
    pub fn with_extra_definers(extra: &[String]) -> TreesitPlugin {
        TreesitPlugin::with_extra_definers(Box::new(ElispSpec), extra)
    }
}

/// Curated (non-exhaustive) set of Emacs Lisp special forms and common
/// built-ins, used by project analysis to suppress unresolved-call warnings.
fn is_elisp_global(name: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let set = SET.get_or_init(|| {
        [
            // special forms / core macros
            "if",
            "when",
            "unless",
            "when-let",
            "when-let*",
            "if-let",
            "if-let*",
            "cond",
            "and",
            "or",
            "not",
            "while",
            "dolist",
            "dotimes",
            "let",
            "let*",
            "letrec",
            "lambda",
            "function",
            "quote",
            "progn",
            "prog1",
            "prog2",
            "setq",
            "setq-default",
            "set",
            "setf",
            "push",
            "pop",
            "incf",
            "decf",
            "cl-incf",
            "cl-decf",
            "save-excursion",
            "save-restriction",
            "save-match-data",
            "with-current-buffer",
            "condition-case",
            "unwind-protect",
            "catch",
            "throw",
            "ignore-errors",
            "ignore",
            "interactive",
            "declare",
            "defvar",
            "defconst",
            "defcustom",
            "defun",
            "defmacro",
            "defsubst",
            "cl-defun",
            "require",
            "provide",
            "eval-when-compile",
            "eval-and-compile",
            "with-eval-after-load",
            "pcase",
            "pcase-let",
            "cl-case",
            "cl-loop",
            "cl-letf",
            "cl-flet",
            "apply",
            "funcall",
            "mapcar",
            "mapc",
            "mapcan",
            "mapconcat",
            "cl-remove-if",
            "cl-remove-if-not",
            "seq-map",
            "seq-filter",
            "seq-reduce",
            "seq-find",
            "seq-do", // list / sequence builtins
            "car",
            "cdr",
            "caar",
            "cadr",
            "cddr",
            "cons",
            "list",
            "append",
            "nth",
            "nthcdr",
            "length",
            "reverse",
            "nreverse",
            "member",
            "memq",
            "assoc",
            "assq",
            "delete",
            "delq",
            "elt",
            "aref",
            "aset",
            "vconcat",
            "vector",
            "make-list",
            "make-vector",
            "last",
            "butlast", // predicates / equality
            "eq",
            "eql",
            "equal",
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
            // arithmetic / strings
            "+",
            "-",
            "*",
            "/",
            "%",
            "mod",
            "1+",
            "1-",
            "max",
            "min",
            "abs",
            "expt",
            "floor",
            "ceiling",
            "=",
            "/=",
            "<",
            ">",
            "<=",
            ">=",
            "concat",
            "format",
            "format-message",
            "string",
            "substring",
            "string=",
            "string<",
            "string-equal",
            "string-match",
            "string-prefix-p",
            "string-suffix-p",
            "string-empty-p",
            "string-blank-p",
            "string-search",
            "string-replace",
            "downcase",
            "upcase",
            "capitalize",
            "split-string",
            "string-join",
            "string-trim",
            "number-to-string",
            "string-to-number",
            "symbol-name",
            "symbol-value",
            "intern",
            "make-symbol",
            "gensym", // io / messaging
            "message",
            "error",
            "user-error",
            "princ",
            "print",
            "prin1",
            "insert",
            "point",
            "goto-char", // hash tables / alist
            "make-hash-table",
            "gethash",
            "puthash",
            "remhash",
            "maphash",
            "hash-table-count",
            "add-to-list",
            "alist-get",
            "plist-get",
            "plist-put",
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
        let content = "(defun foo ()\n  (+ 1 2))\n(provide 'foo)\n";
        assert!(ElispPlugin::new().check_file(content).is_ok());
    }

    #[test]
    fn check_invalid_unbalanced() {
        let content = "(defun foo ()\n  (+ 1 2\n";
        assert!(ElispPlugin::new().check_file(content).is_err());
    }

    #[test]
    fn bounds_beta() {
        let content = "(defun alpha () 1)\n\n(defun beta () 2)\n";
        let bounds = ElispPlugin::new().node_bounds(content, "beta").unwrap();
        assert!(bounds.0 < bounds.1);
    }

    #[test]
    fn replace_defun() {
        let content = "(defun old-f ()\n  1)\n(provide 'x)\n";
        let new_body = "(defun old-f ()\n  2)\n";
        let updated = replace_node(&ElispPlugin::new(), content, "old-f", new_body).unwrap();
        assert!(updated.contains("2)"));
        assert!(ElispPlugin::new().check_file(&updated).is_ok());
    }

    #[test]
    fn insert_after_form() {
        let content = "(defun first () 1)\n(provide 'x)\n";
        let form = "(defun second () 2)";
        let updated = insert_after(&ElispPlugin::new(), content, "first", form).unwrap();
        assert!(updated.contains("defun second"));
        assert!(ElispPlugin::new().check_file(&updated).is_ok());
    }

    #[test]
    fn insert_at_start() {
        let updated =
            insert_after(&ElispPlugin::new(), "", "__start__", "(defun first () 1)").unwrap();
        let nonempty = insert_after(
            &ElispPlugin::new(),
            "(defun existing () 0)\n",
            "__start__",
            "(defun first () 1)",
        )
        .unwrap();
        assert!(
            nonempty.starts_with("(defun first () 1)"),
            "__start__ on nonempty should prepend: {nonempty}"
        );
        assert!(
            nonempty.contains("(defun existing () 0)"),
            "existing form must remain: {nonempty}"
        );
        assert!(updated.contains("defun first"));
    }

    #[test]
    fn insert_at_end() {
        let content = "(defun first () 1)\n";
        let updated =
            insert_after(&ElispPlugin::new(), content, "__end__", "(provide 'x)").unwrap();
        assert!(updated.contains("provide"));
    }

    #[test]
    fn check_node_with_trailing_line_comment() {
        assert!(ElispPlugin::new().check_node("(foo) ; trailing").is_ok());
    }

    #[test]
    fn outline_labels() {
        let content = "(defun a () 1)\n(defvar b 2)\n(defconst c 3)\n";
        let tree = ElispPlugin::new().outline(content).unwrap();
        assert!(tree.contains("defun:a"));
        assert!(tree.contains("defvar:b"));
        assert!(tree.contains("defconst:c"));
    }

    #[test]
    fn documented_defvar_not_flagged() {
        let warnings =
            ElispPlugin::new().semantic_check("(defvar my-var 1 \"A documented var.\")\n");
        assert!(
            !warnings.iter().any(|w| w.contains("missing docstring")),
            "documented defvar should not warn: {warnings:?}"
        );
    }
}
