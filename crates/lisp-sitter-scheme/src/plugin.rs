use lisp_sitter_core::definers::Definer;
use lisp_sitter_core::treesit_plugin::{DialectSpec, TreesitPlugin};
use lisp_sitter_core::FormInfo;

use crate::treesit::base_definers;

/// Scheme dialect specification.
pub struct SchemeSpec;

impl DialectSpec for SchemeSpec {
    fn id(&self) -> &'static str {
        "scheme"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &[".scm", ".ss", ".sld"]
    }

    fn root_kind(&self) -> &'static str {
        "program"
    }

    fn language(&self) -> tree_sitter::Language {
        tree_sitter_scheme::LANGUAGE.into()
    }

    fn base_definers(&self) -> Vec<Definer> {
        base_definers()
    }

    fn definition_template(&self, name: &str, param_list: &str, body: &str) -> String {
        format!("(define ({name} {param_list})\n  {body})\n")
    }

    fn is_known_global(&self, name: &str) -> bool {
        is_scheme_global(name)
    }

    fn semantic_check(&self, content: &str, forms: &[FormInfo]) -> Vec<String> {
        let mut warnings = Vec::new();

        // ── check: missing docstrings for function defines ──────
        for f in forms {
            if f.label.starts_with("define:")
                && !f.label.starts_with("define-library:")
                && f.label.split(':').nth(1).is_some_and(|n| !n.is_empty())
            {
                let text = &content[f.start..f.end];
                // Only function defines (define (foo ...) ...), not simple
                // assignments (define foo 42).
                if text.trim().starts_with("(define (") && !lisp_sitter_core::has_docstring(text) {
                    warnings.push(format!(
                        "{}: missing docstring",
                        lisp_sitter_core::pos_label(content, f.start, &f.label)
                    ));
                }
            }
        }

        // ── check: define-library without (export …) ─────────────
        let has_library = forms.iter().any(|f| f.label.starts_with("define-library"));
        let has_export = content.contains("(export ");
        if has_library && !has_export {
            warnings.push(format!(
                "{}: define-library present but no (export …) form found",
                lisp_sitter_core::pos_label(content, 0, "top")
            ));
        }

        // ── suggest: library wrapper for multiple top-level defines ─
        let library_defines = forms
            .iter()
            .filter(|f| f.label.starts_with("define-library"))
            .count();
        let non_library_defines = forms.len() - library_defines;
        if non_library_defines > 1 && library_defines == 0 {
            warnings.push(format!(
                "{}: {} top-level definitions without a (define-library …) wrapper; consider adding one",
                lisp_sitter_core::pos_label(content, 0, "top"),
                non_library_defines
            ));
        }

        warnings
    }
}

/// The Scheme plugin: a [`TreesitPlugin`] driven by [`SchemeSpec`].
pub struct SchemePlugin;

impl SchemePlugin {
    #[allow(clippy::new_ret_no_self)] // `SchemePlugin` is a constructor namespace
    pub fn new() -> TreesitPlugin {
        TreesitPlugin::new(Box::new(SchemeSpec))
    }

    pub fn with_extra_definers(extra: &[String]) -> TreesitPlugin {
        TreesitPlugin::with_extra_definers(Box::new(SchemeSpec), extra)
    }
}

/// Curated (non-exhaustive) set of Scheme (R7RS-ish) syntactic keywords and
/// common procedures, used by project analysis to suppress unresolved-call
/// warnings.
fn is_scheme_global(name: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let set = SET.get_or_init(|| {
        [
            // syntax / special forms
            "define",
            "define-syntax",
            "define-values",
            "define-record-type",
            "lambda",
            "let",
            "let*",
            "letrec",
            "letrec*",
            "let-values",
            "let*-values",
            "if",
            "cond",
            "case",
            "when",
            "unless",
            "and",
            "or",
            "not",
            "begin",
            "do",
            "delay",
            "force",
            "quote",
            "quasiquote",
            "unquote",
            "set!",
            "else",
            "=>",
            "syntax-rules",
            "parameterize",
            "guard",
            "dynamic-wind",
            "values",
            "call-with-values",
            "call/cc",
            "call-with-current-continuation",
            "apply",
            "map",
            "for-each",
            "filter",
            "fold-left",
            "fold-right",
            "reduce",
            "vector-map",
            "vector-for-each", // pairs / lists
            "car",
            "cdr",
            "caar",
            "cadr",
            "cddr",
            "caddr",
            "cons",
            "list",
            "list*",
            "append",
            "reverse",
            "length",
            "list-ref",
            "list-tail",
            "member",
            "memq",
            "memv",
            "assoc",
            "assq",
            "assv",
            "null?",
            "pair?",
            "list?",
            "set-car!",
            "set-cdr!",
            "last-pair",
            "cons*",
            // predicates / equality
            "eq?",
            "eqv?",
            "equal?",
            "zero?",
            "positive?",
            "negative?",
            "odd?",
            "even?",
            "number?",
            "integer?",
            "string?",
            "symbol?",
            "procedure?",
            "boolean?",
            "char?",
            "vector?",
            "eof-object?", // arithmetic / strings
            "+",
            "-",
            "*",
            "/",
            "modulo",
            "remainder",
            "quotient",
            "abs",
            "min",
            "max",
            "expt",
            "sqrt",
            "floor",
            "ceiling",
            "round",
            "truncate",
            "=",
            "<",
            ">",
            "<=",
            ">=",
            "1+",
            "add1",
            "sub1",
            "number->string",
            "string->number",
            "string-append",
            "string-length",
            "substring",
            "string=?",
            "string<?",
            "string->symbol",
            "symbol->string",
            "string->list",
            "list->string",
            "string-ref",
            "make-string",
            "string",
            "char->integer",
            "integer->char", // vectors / io
            "vector",
            "make-vector",
            "vector-ref",
            "vector-set!",
            "vector-length",
            "vector->list",
            "list->vector",
            "display",
            "write",
            "newline",
            "read",
            "error",
            "raise",
            "exit",
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
        let content = "(define foo 1)\n(define (bar x) (+ x 1))\n";
        assert!(SchemePlugin::new().check_file(content).is_ok());
    }

    #[test]
    fn replace_define() {
        let content = "(define old-f 1)\n";
        let new_body = "(define old-f 2)\n";
        let updated = replace_node(&SchemePlugin::new(), content, "old-f", new_body).unwrap();
        assert!(updated.contains("2)"));
    }

    #[test]
    fn insert_after_define() {
        let content = "(define first 1)\n";
        let form = "(define second 2)";
        let updated = insert_after(&SchemePlugin::new(), content, "first", form).unwrap();
        assert!(updated.contains("define second"));
    }

    #[test]
    fn outline_labels() {
        let content = "(define a 1)\n(define-syntax b (syntax-rules () ((_ x) x)))\n";
        let tree = SchemePlugin::new().outline(content).unwrap();
        assert!(tree.contains("define:a"));
        assert!(tree.contains("define-syntax:b"));
    }

    #[test]
    fn bounds_bar() {
        let content = "(define foo 1)\n(define (bar x) (+ x 1))\n";
        let bounds = SchemePlugin::new().node_bounds(content, "bar").unwrap();
        assert!(bounds.0 < bounds.1);
    }
}
