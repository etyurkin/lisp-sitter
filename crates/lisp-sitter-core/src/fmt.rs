//! Lisp indenter: adjusts leading whitespace to reflect the structure of the
//! enclosing s-expressions.
//!
//! Two modes are supported:
//!
//! 1. **Depth‑based** (default) — each line gets `depth × 2` leading spaces.
//! 2. **Align‑to‑first‑argument** — continuation lines are indented to the
//!    column of the first argument of the containing list, which matches
//!    the convention used by most Lisp editors (`emacs`, `paredit`, …).
//!
//! In both modes strings, line‑comments, block‑comments, sexp‑comments,
//! vectors, char literals (`?\` elisp, `#\` Common Lisp / Scheme), and
//! pipe‑escaped symbols are handled correctly so parens inside them don't
//! affect the indentation count.

use crate::sexp_reader::{self, Dialect};

/// Byte length of the UTF-8 character whose leading byte is `b`.
fn char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// Re-indent `source` so every line's leading whitespace equals `depth × 2`.
pub fn format_source(source: &str) -> String {
    format_source_in(source, Dialect::Generic)
}

/// Like [`format_source`], honoring `dialect`'s char-literal syntax so a paren
/// inside `?\(` / `#\(` is not counted toward indentation depth.
pub fn format_source_in(source: &str, dialect: Dialect) -> String {
    format_inner(source, dialect, false)
}

/// Re-indent with alignment: continuation lines in a multi-line form are
/// indented to the column of the first argument, not just `depth × 2`.
pub fn format_source_aligned(source: &str) -> String {
    format_aligned_in(source, Dialect::Generic)
}

/// Like `format_source_aligned`, with dialect-aware char literals.
pub fn format_aligned_in(source: &str, dialect: Dialect) -> String {
    format_inner(source, dialect, true)
}

/// Indent engine — the `align` flag selects depth-only vs column-aligned style.
fn format_inner(source: &str, dialect: Dialect, align: bool) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;

    // -- scanner state --
    let mut depth: u32 = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    // Column stack for alignment mode: for each depth level, the preferred
    // column for continuation lines. Depth-0 is always 0.
    #[allow(unused_mut)]
    let mut columns: Vec<u32> = Vec::new();

    // Current display column (number of chars on the current line so far).
    let mut col: u32 = 0;

    while i < bytes.len() {
        // ── non-ASCII: copy the whole UTF-8 char verbatim ───────
        // (no multibyte byte is ever syntactically significant, and copying it
        // byte-by-byte as `char` would corrupt it)
        if bytes[i] >= 0x80 {
            let l = char_len(bytes[i]);
            let end = (i + l).min(bytes.len());
            out.push_str(&source[i..end]);
            col += 1;
            i = end;
            continue;
        }

        // ── line / blank-line handling ──────────────────────────
        if bytes[i] == b'\n' {
            out.push('\n');
            col = 0;
            i += 1;
            // Skip leading whitespace on the next line
            while i < bytes.len() && bytes[i].is_ascii_whitespace() && bytes[i] != b'\n' {
                i += 1;
            }
            // Emit the new indent (if there is content on this line)
            if i < bytes.len() && bytes[i] != b'\n' {
                let indent: usize = if align {
                    // Use the anchor column for depth-1 (enclosing form's first arg),
                    // or fall back to depth*2.
                    let d = depth as usize;
                    if d > 0 && d <= columns.len() && columns[d - 1] > 0 {
                        columns[d - 1] as usize
                    } else {
                        d.saturating_mul(2)
                    }
                } else {
                    (depth as usize).saturating_mul(2)
                };
                for _ in 0..indent {
                    out.push(' ');
                }
                col = indent as u32;
            }
            continue;
        }

        // ── in a line comment ──────────────────────────────────
        if in_line_comment {
            out.push(bytes[i] as char);
            col += 1;
            i += 1;
            // A comment's `)` doesn't affect depth, and the loop at
            // the top handles the trailing newline.
            continue;
        }

        // ── in a string ─────────────────────────────────────────
        if in_string {
            out.push(bytes[i] as char);
            col += 1;
            if bytes[i] == b'\\' {
                i += 1;
                if i < bytes.len() {
                    out.push(bytes[i] as char);
                    col += 1;
                    i += 1;
                }
                continue;
            }
            if bytes[i] == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }


        // ── in a block comment ──────────────────────────────────
        if in_block_comment {
            out.push(bytes[i] as char);
            col += 1;
            if bytes[i] == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                out.push(b'#' as char);
                col += 1;
                i += 2;
                in_block_comment = false;
            } else {
                i += 1;
            }
            continue;
        }

        // ── regular character ───────────────────────────────────
        match bytes[i] {
            b'(' => {
                let entering_depth = depth;
                depth = depth.saturating_add(1);
                out.push('(');
                col += 1;
                i += 1;

                // Alignment mode: compute anchor column for continuation
                // lines at this new depth. For "call" forms (not defun/define
                // etc.), the anchor is the column of the first argument after
                // the head symbol. For definitions, we use depth*2 fallback.
                if align {
                    let start_of_form = col; // col right after '('
                    let mut j = i;
                    // Skip whitespace after '('
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() && bytes[j] != b'\n' {
                        j += 1;
                    }
                    // Read the head symbol (keyword) and remember it
                    let head_start = j;
                    while j < bytes.len() && !bytes[j].is_ascii_whitespace()
                        && bytes[j] != b'\n' && bytes[j] != b')' && bytes[j] != b'('
                    {
                        j += 1;
                    }
                    let head = &source[head_start..j];
                    // Skip whitespace to find the first argument
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() && bytes[j] != b'\n' {
                        j += 1;
                    }
                    // Definitions / special forms → no alignment (body at depth*2).
                    let is_def = head.starts_with("def") || matches!(head, "define" | "define-syntax" | "define-record-type" | "lambda" | "let" | "let*");
                    // If the first arg is on the same line, use its column
                    // as the anchor for call forms.
                    if j < bytes.len() && bytes[j] != b'\n' && bytes[j] != b')'
                        && !is_def && !head.is_empty()
                    {
                        // Anchor = column of the first argument
                        let anchor = start_of_form + (j - i) as u32;
                        let d = entering_depth as usize;
                        while columns.len() <= d {
                            columns.push(0);
                        }
                        columns[d] = anchor;
                    } else if entering_depth as usize + 1 > columns.len() {
                        while columns.len() <= entering_depth as usize + 1 {
                            columns.push(0);
                        }
                    }
                }
            }
            b')' => {
                depth = depth.saturating_sub(1);
                out.push(')');
                col += 1;
                i += 1;
                // Alignment: pop column stack when exiting a depth
                if align {
                    let d = depth as usize;
                    if d < columns.len() {
                        columns.truncate(d);
                    }
                }
            }
            b'"' => {
                in_string = true;
                out.push('"');
                col += 1;
                i += 1;
            }
            b';' => {
                in_line_comment = true;
                out.push(';');
                col += 1;
                i += 1;
            }
            // #\c char literal (CL/Scheme) — copy verbatim; the literal char
            // (which may be `(` or `)`) must not affect depth.
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b'\\' => {
                out.push_str(&source[i..i + 2]);
                col += 2;
                i += 2;
                if i < bytes.len() {
                    let l = char_len(bytes[i]);
                    let end = (i + l).min(bytes.len());
                    out.push_str(&source[i..end]);
                    col += 1;
                    i = end;
                }
            }
            // ?c / ?\c char literal (elisp) — copy verbatim, don't count parens.
            b'?' if dialect == Dialect::Elisp => {
                out.push('?');
                col += 1;
                i += 1;
                if i < bytes.len() && bytes[i] == b'\\' {
                    out.push('\\');
                    col += 1;
                    i += 1;
                }
                if i < bytes.len() {
                    let l = char_len(bytes[i]);
                    let end = (i + l).min(bytes.len());
                    out.push_str(&source[i..end]);
                    col += 1;
                    i = end;
                }
            }
            // #; — sexp comment: copy the skipped form verbatim
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b';' => {
                out.push_str(&source[i..i + 2]);
                col += 2;
                i += 2;
                let end = sexp_reader::skip_sexp_in(bytes, i, dialect).unwrap_or(bytes.len());
                out.push_str(&source[i..end]);
                col += (end - i) as u32;
                i = end;
            }
            // #| block comment
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b'|' => {
                in_block_comment = true;
                out.push('#');
                out.push('|');
                col += 2;
                i += 2;
            }
            // #( vector — contributes to depth for indentation
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                depth = depth.saturating_add(1);
                out.push_str(&source[i..i + 2]);
                col += 2;
                i += 2;
            }
            // Pipe-escaped symbol |…| — pass through as-is
            b'|' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'|' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // include closing |
                }
                let slice = &source[start..i];
                out.push_str(slice);
                col += slice.chars().count() as u32; // display columns ≈ chars
            }
            _ => {
                out.push(bytes[i] as char);
                col += 1;
                i += 1;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_kept() {
        let src = "(defun foo (x) x)\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn body_indented() {
        let src = "(defun foo (x)\n  x)\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn fixes_bad_indent() {
        let src = "(defun foo (x)\n      x)\n";
        let want = "(defun foo (x)\n  x)\n";
        assert_eq!(format_source(src), want);
    }

    #[test]
    fn two_levels() {
        let src = "(defun foo (x)\n  (let ((y 1))\n    y))\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn fixes_two_levels() {
        let src = "(defun foo (x)\n  (let ((y 1))\n  y))\n";
        let want = "(defun foo (x)\n  (let ((y 1))\n    y))\n";
        assert_eq!(format_source(src), want);
    }

    #[test]
    fn string_with_parens() {
        let src = "(defun foo ()\n  (message \"(hello)\"))\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn comment_not_counted() {
        let src = "(defun foo ()\n  ;; (fake paren\n  x)\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn sexp_comment() {
        let src = "(defun foo ()\n  #;(dead ()\n  x)\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn sexp_comment_top_level() {
        assert_eq!(format_source("#;(dead)\n(foo)\n"), "#;(dead)\n(foo)\n");
    }

    #[test]
    fn block_comment() {
        let src = "(defun foo ()\n  #| block |#\n  x)\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn blank_lines_preserved() {
        let src = "(defun a () 1)\n\n(defun b () 2)\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn pipe_symbol() {
        let src = "(defun |some name| ()\n  |body|)\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn vector_inside() {
        let src = "(foo\n  #(1 2 3))\n";
        assert_eq!(format_source(src), src);
    }

    // ── aligned formatter tests ─────────────────────────────────────

    #[test]
    fn aligned_simple_body() {
        let src = "(defun foo (x)\n  (+ x 1))\n";
        assert_eq!(format_source_aligned(src), src, "simple defun unchanged");
    }

    #[test]
    fn aligned_let_bindings() {
        // Nested binding list ((x 1) (y 2)) creates two depth levels.
        // (y 2) is at actual depth 2 → 4-space indent from depth*2.
        let solved = "(let ((x 1)\n    (y 2))\n  (+ x y))\n";
        assert_eq!(format_source_aligned("(let ((x 1)\n  (y 2))\n  (+ x y))\n"), solved,
            "deeper continuation aligns to actual depth");
    }

    #[test]
    fn aligned_call_form() {
        // Function-call args align to the first argument, not depth*2
        let src = "(defun foo (x)\n  (format \"result = %s\"\n          x))\n";
        assert_eq!(format_source_aligned(src), src, "regular call args aligned");
    }

    #[test]
    fn aligned_fn_args() {
        // Function call: continuation aligns to first argument column
        let src = "(defun foo (x)\n  (message \"Hello %s\"\n           x))\n";
        assert_eq!(format_source_aligned(src), src, "fn args aligned to first arg");
    }

    #[test]
    fn aligned_depth_fallback() {
        // No first arg on same line → depth*2 fallback
        let src = "(defun foo ()\n  x)\n";
        assert_eq!(format_source_aligned(src), src, "fallback to depth*2");
    }

    #[test]
    fn aligned_deep_nesting() {
        let src = "(defun root (x)\n  (let ((y 1))\n    (+ x y)))\n";
        assert_eq!(format_source_aligned(src), src, "multi-level alignment");
    }

    #[test]
    fn aligned_fixes_misindent_to_align() {
        // depth-based would give depth*2 = 4 spaces, but alignment should give
        // column of first arg (after `message  ` = ~20 cols).
        let mis = "(defun foo (x)\n  (message \"hi\"\n    x))\n";
        let want = "(defun foo (x)\n  (message \"hi\"\n           x))\n";
        let got = format_source_aligned(mis);
        assert_eq!(got, want, "should align to first arg col\nwant:\n{want}\ngot:\n{got}");
    }

    #[test]
    fn aligned_vector_inside() {
        let src = "(foo\n  #(1 2 3))\n";
        assert_eq!(format_source_aligned(src), src);
    }
}
