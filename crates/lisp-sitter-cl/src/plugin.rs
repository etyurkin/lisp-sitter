use lisp_sitter_core::treesit_util::{outline_lines, validate_treesit};
use lisp_sitter_core::{DefinerSet, Error, FormInfo, LanguagePlugin, Result};

use crate::treesit::{base_definers, has_parse_errors, top_level_forms};

pub struct CommonLispPlugin {
    definers: DefinerSet,
}

impl CommonLispPlugin {
    pub fn new() -> Self {
        Self { definers: DefinerSet::new(base_definers()) }
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

        // ── check: missing docstrings ───────────────────────────
        for f in &forms {
            let Some(_name) = f.name.as_deref() else { continue };
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
            matches!(label, "defun" | "defmacro" | "defclass" | "defgeneric" | "defmethod" | "defvar" | "defparameter" | "defstruct")
        });
        if defines_something && !has_in_package {
            warnings.push(format!(
                "{}: file defines symbols but has no (in-package …) form",
                lisp_sitter_core::treesit_util::pos_label(content, 0, "top")
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
        let bounds = CommonLispPlugin::new().node_bounds(content, "beta").unwrap();
        assert!(bounds.0 < bounds.1);
    }
}
