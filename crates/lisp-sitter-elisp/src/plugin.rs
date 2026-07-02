use lisp_sitter_core::treesit_util::outline_lines;
use lisp_sitter_core::{DefinerSet, Error, FormInfo, LanguagePlugin, Result};

use crate::treesit::{base_definers, has_parse_errors, top_level_forms};

pub struct ElispPlugin {
    definers: DefinerSet,
}

impl ElispPlugin {
    /// Plugin with the built-in Emacs Lisp definer set.
    pub fn new() -> Self {
        Self {
            definers: DefinerSet::new(base_definers()),
        }
    }

    /// Plugin whose definer set also recognizes the given extra keywords
    /// (user-configured project def-macros).
    pub fn with_extra_definers(extra: &[String]) -> Self {
        let mut definers = DefinerSet::new(base_definers());
        definers.extend_keywords(extra);
        Self { definers }
    }
}

impl Default for ElispPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguagePlugin for ElispPlugin {
    fn id(&self) -> &'static str {
        "elisp"
    }

    fn extensions(&self) -> &[&'static str] {
        &[".el"]
    }

    fn top_level_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content, &self.definers))
    }

    fn list_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content, &self.definers))
    }

    fn check_file(&self, content: &str) -> Result<()> {
        validate_content(content)
    }

    fn check_node(&self, node: &str) -> Result<()> {
        let wrapped = format!("(progn {})", node.trim());
        validate_content(&wrapped)
    }

    fn outline(&self, content: &str) -> Result<String> {
        let forms = top_level_forms(content, &self.definers);
        if forms.is_empty() && !content.trim().is_empty() {
            validate_content(content)?;
        }
        outline_lines(content, &forms)
    }

    fn tree_depth(&self, content: &str, depth: usize) -> Result<String> {
        let Some(tree) = crate::treesit::parse(content) else {
            return Ok(String::new());
        };
        Ok(lisp_sitter_core::treesit_util::recursive_outline(
            content,
            tree.root_node(),
            depth,
        ))
    }

    fn node_bounds(&self, content: &str, symbol: &str) -> Result<(usize, usize)> {
        let target = symbol.trim();
        crate::treesit::node_bounds(content, &self.definers, target)
            .ok_or_else(|| Error::FormNotFound(target.to_string()))
    }

    fn semantic_check(&self, content: &str) -> Vec<String> {
        let mut warnings = Vec::new();
        let forms = top_level_forms(content, &self.definers);

        // ── check: missing docstrings ───────────────────────────
        for f in &forms {
            let Some(_name) = f.name.as_deref() else {
                continue;
            };
            let text = &content[f.start..f.end];
            let is_def = matches!(
                f.label.split(':').next().unwrap_or(""),
                "defun" | "defsubst" | "cl-defun" | "defmacro"
            );
            if is_def && !lisp_sitter_core::has_docstring(text) {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::treesit_util::pos_label(content, f.start, &f.label)
                ));
            }
            let is_defvar = matches!(
                f.label.split(':').next().unwrap_or(""),
                "defvar" | "defconst" | "defcustom"
            );
            if is_defvar && !lisp_sitter_core::has_docstring(text) && !text.contains("&define") {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::position::pos_label(content, f.start, &f.label)
                ));
            }
        }

        // ── check: missing (provide '…) ─────────────────────────
        let has_provide = content.contains("(provide ");
        let defines_something = forms.iter().any(|f| {
            let label = f.label.split(':').next().unwrap_or("");
            matches!(
                label,
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
                lisp_sitter_core::position::pos_label(content, 0, "top")
            ));
        }

        warnings
    }
    fn form_body_range(&self, form_text: &str) -> Option<(usize, usize)> {
        let tree = crate::treesit::parse(form_text)?;
        let info = lisp_sitter_core::treesit_util::analyze_def_form(form_text, tree.root_node())?;
        Some((info.body_start, info.body_end))
    }

    fn form_params_and_body(&self, form_text: &str) -> Option<(Vec<String>, String)> {
        let tree = crate::treesit::parse(form_text)?;
        let info = lisp_sitter_core::treesit_util::analyze_def_form(form_text, tree.root_node())?;
        Some((
            info.param_names,
            form_text[info.body_start..info.body_end].to_string(),
        ))
    }

    fn form_rename_name(&self, form_text: &str, old: &str, new: &str) -> Option<String> {
        let tree = crate::treesit::parse(form_text)?;
        let info = lisp_sitter_core::treesit_util::analyze_def_form(form_text, tree.root_node())?;
        if &form_text[info.name_start..info.name_end] != old {
            return None;
        }
        let mut result = form_text.to_string();
        result.replace_range(info.name_start..info.name_end, new);
        Some(result)
    }

    fn find_sexp_in(&self, content: &str, pattern: &str) -> Option<Option<(usize, usize)>> {
        let tree = crate::treesit::parse(content)?;
        Some(lisp_sitter_core::treesit_util::find_sexp_in_tree(
            content,
            pattern,
            tree.root_node(),
        ))
    }

    fn find_symbol_refs(
        &self,
        content: &str,
        symbol: &str,
    ) -> Vec<lisp_sitter_core::plugin::SymbolRef> {
        crate::treesit::parse(content)
            .map(|tree| {
                lisp_sitter_core::treesit_util::find_symbol_refs_in_tree(
                    content,
                    tree.root_node(),
                    symbol,
                )
            })
            .unwrap_or_default()
    }

    fn find_errors(&self, content: &str) -> Vec<String> {
        crate::treesit::parse(content)
            .map(|tree| lisp_sitter_core::treesit_util::find_error_nodes(content, tree.root_node()))
            .unwrap_or_default()
    }

    fn is_known_global(&self, name: &str) -> bool {
        is_elisp_global(name)
    }
} // impl LanguagePlugin for ElispPlugin

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
            "seq-do",
            // list / sequence builtins
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
            "butlast",
            // predicates / equality
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
            "split-string",
            "string-join",
            "string-trim",
            "number-to-string",
            "string-to-number",
            "symbol-name",
            "symbol-value",
            "intern",
            "make-symbol",
            "gensym",
            // io / messaging
            "message",
            "error",
            "user-error",
            "princ",
            "print",
            "prin1",
            "insert",
            "point",
            "goto-char",
            // hash tables / alist
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

fn validate_content(content: &str) -> Result<()> {
    if let Some(err) =
        lisp_sitter_core::scan::scan_parens_in(content, lisp_sitter_core::Dialect::Elisp)
    {
        return Err(Error::Syntax(err));
    }
    if has_parse_errors(content) {
        return Err(Error::Syntax(lisp_sitter_core::position::error_at(
            content,
            0,
            "tree-sitter parse error",
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lisp_sitter_core::edit::{insert_after, replace_node};

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
        let rendered = format!("{}:{}", bounds.0, bounds.1);
        assert!(rendered.chars().any(|c| c.is_ascii_digit()));
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
    fn outline_labels() {
        let content = "(defun a () 1)\n(defvar b 2)\n(defconst c 3)\n";
        let tree = ElispPlugin::new().outline(content).unwrap();
        assert!(tree.contains("defun:a"));
        assert!(tree.contains("defvar:b"));
        assert!(tree.contains("defconst:c"));
    }
}
