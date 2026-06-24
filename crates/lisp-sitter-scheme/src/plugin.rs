use lisp_sitter_core::treesit_util::{outline_lines, validate_treesit};
use lisp_sitter_core::{DefinerSet, Error, FormInfo, LanguagePlugin, Result};

use crate::treesit::{base_definers, has_parse_errors, top_level_forms};

pub struct SchemePlugin {
    definers: DefinerSet,
}

impl SchemePlugin {
    pub fn new() -> Self {
        Self { definers: DefinerSet::new(base_definers()) }
    }

    pub fn with_extra_definers(extra: &[String]) -> Self {
        let mut definers = DefinerSet::new(base_definers());
        definers.extend_keywords(extra);
        Self { definers }
    }
}

impl Default for SchemePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguagePlugin for SchemePlugin {
    fn id(&self) -> &'static str {
        "scheme"
    }

    fn extensions(&self) -> &[&'static str] {
        &[".scm", ".ss", ".sld"]
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
        Ok(
            lisp_sitter_core::treesit_util::recursive_outline(content, tree.root_node(), depth),
        )
    }

    fn node_bounds(&self, content: &str, symbol: &str) -> Result<(usize, usize)> {
        let target = symbol.trim();
        crate::treesit::node_bounds(content, &self.definers, target)
            .ok_or_else(|| Error::FormNotFound(target.to_string()))
    }

    fn semantic_check(&self, content: &str) -> Vec<String> {
        let mut warnings = Vec::new();
        let forms = top_level_forms(content, &self.definers);

        // ── check: missing docstrings for function defines ──────
        for f in &forms {
            if f.label.starts_with("define:") && !f.label.starts_with("define-library:")
                && f.label.split(':').nth(1).is_some_and(|n| !n.is_empty())
            {
                let text = &content[f.start..f.end];
                // Only check function defines (define (foo ...) ...), not
                // simple assignments (define foo 42).
                if text.trim().starts_with("(define (") && !lisp_sitter_core::has_docstring(text) {
                    warnings.push(format!(
                        "{}: missing docstring",
                        lisp_sitter_core::treesit_util::pos_label(content, f.start, &f.label)
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
                lisp_sitter_core::treesit_util::pos_label(content, 0, "top")
            ));
        }

        // ── suggest: library wrapper for multiple top-level defines ─
        let library_defines = forms.iter().filter(|f| f.label.starts_with("define-library")).count();
        let non_library_defines = forms.len() - library_defines;
        if non_library_defines > 1 && library_defines == 0 {
            warnings.push(format!(
                "{}: {} top-level definitions without a (define-library …) wrapper; consider adding one",
                lisp_sitter_core::treesit_util::pos_label(content, 0, "top"),
                non_library_defines
            ));
        }

        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lisp_sitter_core::edit::{insert_after, replace_node};

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
