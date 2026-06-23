use lisp_sitter_core::treesit_util::outline_lines;
use lisp_sitter_core::{Error, FormInfo, LanguagePlugin, Result};

use crate::treesit::{has_parse_errors, replaceable_label, top_level_forms};

pub struct ElispPlugin;

impl LanguagePlugin for ElispPlugin {
    fn id(&self) -> &'static str {
        "elisp"
    }

    fn extensions(&self) -> &[&'static str] {
        &[".el"]
    }

    fn top_level_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content))
    }

    fn list_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(top_level_forms(content))
    }

    fn check_file(&self, content: &str) -> Result<()> {
        validate_content(content)
    }

    fn check_node(&self, node: &str) -> Result<()> {
        let wrapped = format!("(progn {})", node.trim());
        validate_content(&wrapped)
    }

    fn outline(&self, content: &str) -> Result<String> {
        let forms = top_level_forms(content);
        if forms.is_empty() && !content.trim().is_empty() {
            validate_content(content)?;
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
        for form in top_level_forms(content) {
            let Some(name) = form.name.as_deref() else {
                continue;
            };
            if name != target {
                continue;
            }
            if !replaceable_label(&form.label) {
                continue;
            }
            return Ok((form.start, form.end));
        }
        Err(Error::FormNotFound(target.to_string()))
    }

    fn semantic_check(&self, content: &str) -> Vec<String> {
        let mut warnings = Vec::new();
        let forms = top_level_forms(content);

        // ── check: missing docstrings ───────────────────────────
        for f in &forms {
            let Some(_name) = f.name.as_deref() else { continue };
            let text = &content[f.start..f.end];
            let is_def = matches!(
                f.label.split(':').next().unwrap_or(""),
                "defun" | "defsubst" | "cl-defun" | "defmacro"
            );
            if is_def && !has_docstring(text) {
                warnings.push(format!(
                    "{}: missing docstring",
                    lisp_sitter_core::position::pos_label(content, f.start, &f.label)
                ));
            }
            let is_defvar = matches!(
                f.label.split(':').next().unwrap_or(""),
                "defvar" | "defconst" | "defcustom"
            );
            if is_defvar && !has_docstring(text) && !text.contains("&define") {
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
            matches!(label, "defun" | "defsubst" | "defmacro" | "cl-defun" | "defvar" | "defconst" | "defcustom")
        });
        if defines_something && !has_provide {
            warnings.push(format!(
                "{}: file defines symbols but has no (provide '…) form",
                lisp_sitter_core::position::pos_label(content, 0, "top")
            ));
        }

        warnings
    }
}  // impl LanguagePlugin for ElispPlugin

fn has_docstring(form_text: &str) -> bool {
    // A docstring is a string literal right after the arglist.
    // Quick heuristic: look for `"…"` after the first `)` of the arglist.
    let mut depth = 0i32;
    let mut found_after_args = false;
    let bytes = form_text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => { depth += 1; i += 1; }
            b')' => {
                if depth == 1 {
                    // This `)` closes the arglist (next meaningful thing should be docstring)
                    found_after_args = true;
                }
                depth -= 1;
                i += 1;
            }
            b'"' if found_after_args => {
                // Found a string right after the arglist — that's the docstring
                return true;
            }
            b';' => { while i < bytes.len() && bytes[i] != b'\n' { i += 1; } }
            b'#' if i + 1 < bytes.len() && bytes[i+1] == b'|' => {
                while i + 1 < bytes.len() && !(bytes[i] == b'|' && bytes[i+1] == b'#') { i += 1; }
                if i < bytes.len() { i += 2; }
            }
            _ => { i += 1; }
        }
        if depth < 0 { break; }
    }
    false
}

fn validate_content(content: &str) -> Result<()> {
    if let Some(err) = lisp_sitter_core::scan::scan_parens(content) {
        return Err(Error::Syntax(err));
    }
    if has_parse_errors(content) {
        return Err(Error::Syntax(
            lisp_sitter_core::position::error_at(content, 0, "tree-sitter parse error"),
        ));
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
        assert!(ElispPlugin.check_file(content).is_ok());
    }

    #[test]
    fn check_invalid_unbalanced() {
        let content = "(defun foo ()\n  (+ 1 2\n";
        assert!(ElispPlugin.check_file(content).is_err());
    }

    #[test]
    fn bounds_beta() {
        let content = "(defun alpha () 1)\n\n(defun beta () 2)\n";
        let bounds = ElispPlugin.node_bounds(content, "beta").unwrap();
        assert!(bounds.0 < bounds.1);
        let rendered = format!("{}:{}", bounds.0, bounds.1);
        assert!(rendered.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn replace_defun() {
        let content = "(defun old-f ()\n  1)\n(provide 'x)\n";
        let new_body = "(defun old-f ()\n  2)\n";
        let updated = replace_node(&ElispPlugin, content, "old-f", new_body).unwrap();
        assert!(updated.contains("2)"));
        assert!(ElispPlugin.check_file(&updated).is_ok());
    }

    #[test]
    fn insert_after_form() {
        let content = "(defun first () 1)\n(provide 'x)\n";
        let form = "(defun second () 2)";
        let updated = insert_after(&ElispPlugin, content, "first", form).unwrap();
        assert!(updated.contains("defun second"));
        assert!(ElispPlugin.check_file(&updated).is_ok());
    }

    #[test]
    fn insert_at_start() {
        let updated =
            insert_after(&ElispPlugin, "", "__start__", "(defun first () 1)").unwrap();
        assert!(updated.contains("defun first"));
    }

    #[test]
    fn insert_at_end() {
        let content = "(defun first () 1)\n";
        let updated =
            insert_after(&ElispPlugin, content, "__end__", "(provide 'x)").unwrap();
        assert!(updated.contains("provide"));
    }

    #[test]
    fn outline_labels() {
        let content = "(defun a () 1)\n(defvar b 2)\n(defconst c 3)\n";
        let tree = ElispPlugin.outline(content).unwrap();
        assert!(tree.contains("defun:a"));
        assert!(tree.contains("defvar:b"));
        assert!(tree.contains("defconst:c"));
    }
}
