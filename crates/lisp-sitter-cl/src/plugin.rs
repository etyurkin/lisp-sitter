use lisp_sitter_core::treesit_util::{outline_lines, validate_treesit};
use lisp_sitter_core::{DefinerSet, Error, FormInfo, LanguagePlugin, Result};

use crate::treesit::{base_definers, has_parse_errors, top_level_forms};

pub struct CommonLispPlugin {
    definers: DefinerSet,
}

impl CommonLispPlugin {
    pub fn new() -> Self {
        Self {
            definers: DefinerSet::new(base_definers()),
        }
    }

    pub fn with_extra_definers(extra: &[String]) -> Self {
        let mut definers = DefinerSet::new(base_definers());
        definers.extend_keywords(extra);
        Self { definers }
    }
}

impl Default for CommonLispPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguagePlugin for CommonLispPlugin {
    fn id(&self) -> &'static str {
        "commonlisp"
    }

    fn extensions(&self) -> &[&'static str] {
        &[".lisp", ".cl"]
    }

    fn top_level_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content, &self.definers))
    }

    fn list_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content, &self.definers))
    }

    fn check_file(&self, content: &str) -> Result<()> {
        validate_treesit(content, has_parse_errors(content))
    }

    fn check_node(&self, node: &str) -> Result<()> {
        let wrapped = format!("{}\n", node.trim());
        validate_treesit(&wrapped, has_parse_errors(&wrapped))
    }

    fn outline(&self, content: &str) -> Result<String> {
        let forms = top_level_forms(content, &self.definers);
        if forms.is_empty() && !content.trim().is_empty() {
            validate_treesit(content, has_parse_errors(content))?;
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
            let label = f.label.split(':').next().unwrap_or("");
            let wants_doc = matches!(
                label,
                "defun" | "defmacro" | "defgeneric" | "defmethod" | "defclass"
            );
            if wants_doc && !lisp_sitter_core::has_docstring(text) {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::treesit_util::pos_label(content, f.start, &f.label)
                ));
            }
            let wants_var_doc = matches!(label, "defvar" | "defparameter" | "defconstant");
            if wants_var_doc && !lisp_sitter_core::has_docstring(text) {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::treesit_util::pos_label(content, f.start, &f.label)
                ));
            }
        }

        // ── check: missing (in-package …) ────────────────────────
        let has_in_package = content.contains("(in-package ");
        let defines_something = forms.iter().any(|f| {
            let label = f.label.split(':').next().unwrap_or("");
            matches!(
                label,
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
                lisp_sitter_core::treesit_util::pos_label(content, 0, "top")
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
        is_cl_global(name)
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
            "find-if",
            // list / sequence
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
            "mapcar",
            "every",
            "some",
            "notany",
            // predicates / equality
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
            "values",
            // arithmetic / strings
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
