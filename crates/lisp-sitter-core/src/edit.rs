use crate::anchors::{is_anchor_end, is_anchor_start, ANCHOR_END, ANCHOR_START};
use crate::error::{Error, Result};
use crate::plugin::LanguagePlugin;
use crate::scan::{content_blank, replace_region};
use crate::sexp_reader::{
    at_token_start, skip_atom_in, skip_block_comment, skip_line_comment, skip_sexp_in, skip_string,
    Dialect,
};

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
    // Prefer the plugin's structured forms (for elisp these are definer forms,
    // so a new form lands before a trailing `(provide …)`). Fall back to a
    // byte-level scan of *all* top-level forms so a file made only of
    // non-definer forms — e.g. `(require 'a)\n(provide 'x)` — still appends.
    if let Some(end) = plugin.top_level_forms(content)?.last().map(|f| f.end) {
        return Ok(end);
    }
    last_top_level_form_end(content).ok_or_else(|| Error::Message("No forms".into()))
}

/// Byte offset just past the last complete top-level s-expression, found with
/// the raw scanner (dialect-agnostic; top-level `?(` literals don't occur).
fn last_top_level_form_end(content: &str) -> Option<usize> {
    use crate::sexp_reader::{skip_sexp_in, skip_whitespace_and_comments};
    let b = content.as_bytes();
    let mut i = 0;
    let mut last_end = None;
    loop {
        i = skip_whitespace_and_comments(b, i);
        if i >= b.len() {
            break;
        }
        match skip_sexp_in(b, i, Dialect::Generic) {
            Ok(end) if end > i => {
                last_end = Some(end);
                i = end;
            }
            _ => break,
        }
    }
    last_end
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
            b'"' => {
                i = skip_string(b, i).unwrap_or(b.len());
            }
            b';' => {
                i = skip_line_comment(b, i).unwrap_or(b.len());
            }
            // Block comment `#| ... |#` (possibly nested) — skip as a unit;
            // skip_sexp_in would mis-scan an interior `|` as a pipe symbol and
            // leak the rest of the comment as code.
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => {
                i = skip_block_comment(b, i).unwrap_or(b.len());
            }
            b'#' => {
                i = skip_sexp_in(b, i, dialect).unwrap_or(i + 1);
            }
            // Leading elisp char literal `?(` / `?\(` — skip so its paren is not
            // seen as a call. Mid-symbol `?` (e.g. `foo?`) is an ordinary symbol.
            b'?' if dialect == Dialect::Elisp && at_token_start(b, i) => {
                i = skip_sexp_in(b, i, dialect).unwrap_or(i + 1);
            }
            b'(' => {
                let call_pos = i;
                i += 1;
                let mut ws = i;
                while ws < b.len() && matches!(b[ws], b' ' | b'\t' | b'\n' | b'\r') {
                    ws += 1;
                }
                if let Ok(sym_end) = skip_atom_in(b, ws, dialect) {
                    if sym_end > ws && &content[ws..sym_end] == sym {
                        positions.push(call_pos);
                    }
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    positions
}

/// Collect the names of every `(sym …)` call head in `content`, skipping
/// strings, comments, block comments, and char literals. Used as the
/// tree-sitter-free fallback for `LanguagePlugin::referenced_names`.
pub fn call_head_names_in(content: &str, dialect: Dialect) -> std::collections::HashSet<String> {
    let b = content.as_bytes();
    let mut names = std::collections::HashSet::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(b, i).unwrap_or(b.len()),
            b';' => i = skip_line_comment(b, i).unwrap_or(b.len()),
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => {
                i = skip_block_comment(b, i).unwrap_or(b.len());
            }
            b'#' => i = skip_sexp_in(b, i, dialect).unwrap_or(i + 1),
            b'?' if dialect == Dialect::Elisp && at_token_start(b, i) => {
                i = skip_sexp_in(b, i, dialect).unwrap_or(i + 1);
            }
            b'(' => {
                let mut ws = i + 1;
                while ws < b.len() && matches!(b[ws], b' ' | b'\t' | b'\n' | b'\r') {
                    ws += 1;
                }
                if let Ok(end) = skip_atom_in(b, ws, dialect) {
                    if end > ws {
                        if let Ok(name) = std::str::from_utf8(&b[ws..end]) {
                            names.insert(name.to_string());
                        }
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    names
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
        assert_eq!(
            hits.len(),
            2,
            "both (bar) and (bar x) must be found: {hits:?}"
        );
        assert!(hits[0] < hits[1], "outer call must come first");
    }

    #[test]
    fn find_callers_no_false_positive_substring() {
        let src = "(bar-extended x) (xbar y) (bar)";
        let hits = find_callers_in(src, "bar", Dialect::Generic);
        assert_eq!(hits.len(), 1, "only (bar) should match, got {hits:?}");
    }

    #[test]
    fn find_callers_skips_block_comment_with_interior_pipe() {
        // The `(bar)` inside the block comment must not be reported even though
        // the comment contains an odd number of interior `|`.
        let src = "#| a|b (bar) |#\n(bar 1)\n";
        let hits = find_callers_in(src, "bar", Dialect::Generic);
        assert_eq!(hits.len(), 1, "only the real call counts, got {hits:?}");
        assert!(
            hits[0] > 15,
            "the hit must be the real call, not in the comment"
        );
    }

    #[test]
    fn last_form_end_covers_non_definer_forms() {
        let src = "(require 'a)\n(provide 'x)\n";
        let end = last_top_level_form_end(src).unwrap();
        assert_eq!(&src[..end], "(require 'a)\n(provide 'x)");
    }
}
