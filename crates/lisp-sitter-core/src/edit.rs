use crate::anchors::{is_anchor_end, is_anchor_start, ANCHOR_END, ANCHOR_START};
use crate::error::{Error, Result};
use crate::plugin::LanguagePlugin;
use crate::scan::{content_blank, replace_region};
use crate::sexp_reader::{skip_atom_in, skip_line_comment, skip_sexp_in, skip_string, Dialect};

pub fn get_form_text<'a>(
    plugin: &dyn LanguagePlugin,
    content: &'a str,
    symbol: &str,
) -> Result<&'a str> {
    let (start, end) = plugin.node_bounds(content, symbol)?;
    Ok(&content[start..end])
}

/// Refuse a destructive structural edit when the *source* file is already
/// malformed. With unbalanced parens, a form's computed bounds can run to EOF
/// and silently swallow every following form, so editing in place would delete
/// content. New/empty files are well-formed and pass this check.
pub fn ensure_source_editable(plugin: &dyn LanguagePlugin, content: &str) -> Result<()> {
    plugin.check_file(content).map_err(|e| match e {
        Error::Syntax(detail) => Error::MalformedSource(detail),
        other => other,
    })
}

pub fn replace_node(
    plugin: &dyn LanguagePlugin,
    content: &str,
    symbol: &str,
    new_body: &str,
) -> Result<String> {
    let body = new_body.trim();
    if !body.starts_with('(') {
        return Err(Error::BodyNotSexp);
    }
    ensure_source_editable(plugin, content)?;
    let (start, end) = plugin.node_bounds(content, symbol)?;
    let updated = replace_region(content, start, end, body);
    plugin.check_file(&updated).map_err(|e| match e {
        Error::Syntax(detail) => Error::SyntaxAfterEdit {
            operation: "replace".into(),
            detail,
        },
        other => other,
    })?;
    Ok(updated)
}

pub fn insert_after(
    plugin: &dyn LanguagePlugin,
    content: &str,
    after_symbol: &str,
    node: &str,
) -> Result<String> {
    let body = node.trim();
    if body.is_empty() {
        return Err(Error::EmptyForm);
    }
    if !body.starts_with('(') {
        return Err(Error::BodyNotSexp);
    }
    ensure_source_editable(plugin, content)?;

    let pos = find_insert_position(plugin, content, after_symbol)?;
    let blank = content_blank(content);
    let insertion = if pos == 0 && blank {
        body.to_string()
    } else {
        format!("\n\n{body}")
    };
    let updated = replace_region(content, pos, pos, &insertion);
    plugin.check_file(&updated).map_err(|e| match e {
        Error::Syntax(detail) => Error::SyntaxAfterEdit {
            operation: "insert".into(),
            detail,
        },
        other => other,
    })?;
    Ok(updated)
}

fn find_insert_position(
    plugin: &dyn LanguagePlugin,
    content: &str,
    after_symbol: &str,
) -> Result<usize> {
    if is_anchor_start(after_symbol) {
        if content_blank(content) {
            return Ok(0);
        }
        return Err(Error::StartAnchorOnNonempty(
            ANCHOR_START.into(),
            ANCHOR_END.into(),
        ));
    }
    if is_anchor_end(after_symbol) {
        return end_of_forms(plugin, content);
    }
    let (_, end) = plugin.node_bounds(content, after_symbol)?;
    Ok(end)
}

fn end_of_forms(plugin: &dyn LanguagePlugin, content: &str) -> Result<usize> {
    if content_blank(content) {
        return Ok(0);
    }
    plugin
        .top_level_forms(content)?
        .last()
        .map(|f| f.end)
        .ok_or_else(|| Error::Message("No forms".into()))
}

/// Return byte positions of every `(sym …)` or `(sym)` call in `content`,
/// skipping strings, line comments, block comments, and char literals so
/// only syntactically valid call sites are reported.
pub fn find_callers_in(content: &str, sym: &str, dialect: Dialect) -> Vec<usize> {
    let b = content.as_bytes();
    let mut positions = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => { i = skip_string(b, i).unwrap_or(b.len()); }
            b';' => { i = skip_line_comment(b, i).unwrap_or(b.len()); }
            b'#' => { i = skip_sexp_in(b, i, dialect).unwrap_or(i + 1); }
            b'?' if dialect == Dialect::Elisp => { i = skip_sexp_in(b, i, dialect).unwrap_or(i + 1); }
            b'(' => {
                let call_pos = i;
                i += 1;
                let mut ws = i;
                while ws < b.len() && matches!(b[ws], b' ' | b'\t' | b'\n' | b'\r') { ws += 1; }
                if let Ok(sym_end) = skip_atom_in(b, ws, dialect) {
                    if sym_end > ws && &content[ws..sym_end] == sym {
                        positions.push(call_pos);
                    }
                }
            }
            _ => { i += 1; }
        }
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_callers_skips_comments_and_strings() {
        let src = r#"
(defun foo () 1)
; (bar) in a comment — should not be found
(defun test ()
  "(bar) in a string"   ; also not a call
  (bar)                 ; zero-arg call — must be found
  (bar x))              ; arg call after zero-arg — must also be found
"#;
        let hits = find_callers_in(src, "bar", Dialect::Elisp);
        assert_eq!(hits.len(), 2, "expected exactly 2 call sites, got {hits:?}");
    }

    #[test]
    fn find_callers_zero_arg_before_arg_call() {
        let src = "(bar (bar x))";
        let hits = find_callers_in(src, "bar", Dialect::Generic);
        assert_eq!(hits.len(), 2, "both (bar) and (bar x) must be found: {hits:?}");
        assert!(hits[0] < hits[1], "outer call must come first");
    }

    #[test]
    fn find_callers_no_false_positive_substring() {
        let src = "(bar-extended x) (xbar y) (bar)";
        let hits = find_callers_in(src, "bar", Dialect::Generic);
        assert_eq!(hits.len(), 1, "only (bar) should match, got {hits:?}");
    }
}
