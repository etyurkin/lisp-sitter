/// A simple Lisp indenter that adjusts leading whitespace based on paren depth.
///
/// For each line of content the indentation is set to `depth × 2` spaces,
/// where `depth` is the number of unmatched open parentheses at the point
/// where the line's content begins (i.e. after leading whitespace is stripped).
/// Strings and line-comments are respected so parens inside them don't
/// affect the count. Block comments (`#|…|#`) and sexp-comments (`#;`)
/// are also handled correctly.
///
/// This produces readable output for most code. A full
/// align-to-first-argument formatter could sit on top of this foundation.

use crate::sexp_reader;

/// Re-indent `source` so every line's leading whitespace equals `depth × 2`.
pub fn format_source(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;

    // -- scanner state --
    let mut depth: u32 = 0;
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < bytes.len() {
        // ── line / blank-line handling ──────────────────────────
        if bytes[i] == b'\n' {
            out.push('\n');
            i += 1;
            // Skip leading whitespace on the next line
            while i < bytes.len() && bytes[i].is_ascii_whitespace() && bytes[i] != b'\n' {
                i += 1;
            }
            // Emit the new indent (if there is content on this line)
            if i < bytes.len() && bytes[i] != b'\n' {
                let indent = (depth as usize).saturating_mul(2);
                for _ in 0..indent {
                    out.push(' ');
                }
            }
            continue;
        }

        // ── in a line comment ──────────────────────────────────
        if in_line_comment {
            out.push(bytes[i] as char);
            i += 1;
            // A comment's `)` doesn't affect depth, and the loop at
            // the top handles the trailing newline.
            continue;
        }

        // ── in a string ─────────────────────────────────────────
        if in_string {
            out.push(bytes[i] as char);
            if bytes[i] == b'\\' {
                i += 1;
                if i < bytes.len() {
                    out.push(bytes[i] as char);
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
            if bytes[i] == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                out.push(b'#' as char);
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
                depth = depth.saturating_add(1);
                out.push('(');
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                out.push(')');
                i += 1;
            }
            b'"' => {
                in_string = true;
                out.push('"');
                i += 1;
            }
            b';' => {
                in_line_comment = true;
                out.push(';');
                i += 1;
            }
            // #; — sexp comment: copy the skipped form verbatim
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b';' => {
                out.push('#');
                out.push(';');
                i += 2;
                if let Ok(end) = sexp_reader::skip_sexp(bytes, i) {
                    while i < end {
                        out.push(bytes[i] as char);
                        i += 1;
                    }
                } else {
                    // Incomplete — copy to end
                    while i < bytes.len() {
                        out.push(bytes[i] as char);
                        i += 1;
                    }
                }
            }
            // #| block comment
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b'|' => {
                in_block_comment = true;
                out.push('#');
                out.push('|');
                i += 2;
            }
            // #( vector — contributes to depth for indentation
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                depth = depth.saturating_add(1);
                out.push('#');
                out.push('(');
                i += 2;
            }
            // Pipe-escaped symbol |…| — pass through as-is
            b'|' => {
                out.push('|');
                i += 1;
                while i < bytes.len() && bytes[i] != b'|' {
                    out.push(bytes[i] as char);
                    i += 1;
                }
                if i < bytes.len() {
                    out.push('|');
                    i += 1;
                }
            }
            _ => {
                out.push(bytes[i] as char);
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
}
