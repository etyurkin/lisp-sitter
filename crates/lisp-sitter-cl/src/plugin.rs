use lisp_sitter_core::treesit_util::{outline_lines, validate_treesit};
use lisp_sitter_core::{Error, FormInfo, LanguagePlugin, Result};

use crate::treesit::{has_parse_errors, top_level_forms};

pub struct CommonLispPlugin;

impl LanguagePlugin for CommonLispPlugin {
    fn id(&self) -> &'static str {
        "commonlisp"
    }

    fn extensions(&self) -> &[&'static str] {
        &[".lisp", ".cl"]
    }

    fn top_level_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content))
    }

    fn list_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content))
    }

    fn check_file(&self, content: &str) -> Result<()> {
        validate_treesit(content, has_parse_errors(content))
    }

    fn check_node(&self, node: &str) -> Result<()> {
        let wrapped = format!("{}\n", node.trim());
        validate_treesit(&wrapped, has_parse_errors(&wrapped))
    }

    fn outline(&self, content: &str) -> Result<String> {
        let forms = top_level_forms(content);
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
        crate::treesit::node_bounds(content, target)
            .ok_or_else(|| Error::FormNotFound(target.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lisp_sitter_core::edit::{insert_after, replace_node};

    #[test]
    fn check_valid_file() {
        let content = "(defun foo ()\n  (+ 1 2))\n";
        assert!(CommonLispPlugin.check_file(content).is_ok());
    }

    #[test]
    fn replace_defun() {
        let content = "(defun old-f ()\n  1)\n";
        let new_body = "(defun old-f ()\n  2)\n";
        let updated = replace_node(&CommonLispPlugin, content, "old-f", new_body).unwrap();
        assert!(updated.contains("2)"));
        assert!(CommonLispPlugin.check_file(&updated).is_ok());
    }

    #[test]
    fn insert_after_form() {
        let content = "(defun first () 1)\n";
        let form = "(defun second () 2)";
        let updated = insert_after(&CommonLispPlugin, content, "first", form).unwrap();
        assert!(updated.contains("defun second"));
    }

    #[test]
    fn outline_labels() {
        let content = "(defun a () 1)\n(defmacro b () '(+ 1 2))\n";
        let tree = CommonLispPlugin.outline(content).unwrap();
        assert!(tree.contains("defun:a"));
        assert!(tree.contains("defmacro:b"));
    }

    #[test]
    fn bounds_by_name() {
        let content = "(defun alpha () 1)\n(defun beta () 2)\n";
        let bounds = CommonLispPlugin.node_bounds(content, "beta").unwrap();
        assert!(bounds.0 < bounds.1);
    }
}
