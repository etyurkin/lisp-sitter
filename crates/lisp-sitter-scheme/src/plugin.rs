use lisp_sitter_core::treesit_util::{outline_lines, validate_treesit};
use lisp_sitter_core::{Error, FormInfo, LanguagePlugin, Result};

use crate::treesit::{has_parse_errors, top_level_forms_with_library};

pub struct SchemePlugin;

impl LanguagePlugin for SchemePlugin {
    fn id(&self) -> &'static str {
        "scheme"
    }

    fn extensions(&self) -> &[&'static str] {
        &[".scm", ".ss", ".sld"]
    }

    fn top_level_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms_with_library(content))
    }

    fn list_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms_with_library(content))
    }

    fn check_file(&self, content: &str) -> Result<()> {
        validate_treesit(content, has_parse_errors(content))
    }

    fn check_node(&self, node: &str) -> Result<()> {
        let wrapped = format!("{}\n", node.trim());
        validate_treesit(&wrapped, has_parse_errors(&wrapped))
    }

    fn outline(&self, content: &str) -> Result<String> {
        let forms = top_level_forms_with_library(content);
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
        let content = "(define foo 1)\n(define (bar x) (+ x 1))\n";
        assert!(SchemePlugin.check_file(content).is_ok());
    }

    #[test]
    fn replace_define() {
        let content = "(define old-f 1)\n";
        let new_body = "(define old-f 2)\n";
        let updated = replace_node(&SchemePlugin, content, "old-f", new_body).unwrap();
        assert!(updated.contains("2)"));
    }

    #[test]
    fn insert_after_define() {
        let content = "(define first 1)\n";
        let form = "(define second 2)";
        let updated = insert_after(&SchemePlugin, content, "first", form).unwrap();
        assert!(updated.contains("define second"));
    }

    #[test]
    fn outline_labels() {
        let content = "(define a 1)\n(define-syntax b (syntax-rules () ((_ x) x)))\n";
        let tree = SchemePlugin.outline(content).unwrap();
        assert!(tree.contains("define:a"));
        assert!(tree.contains("define-syntax:b"));
    }

    #[test]
    fn bounds_bar() {
        let content = "(define foo 1)\n(define (bar x) (+ x 1))\n";
        let bounds = SchemePlugin.node_bounds(content, "bar").unwrap();
        assert!(bounds.0 < bounds.1);
    }
}
