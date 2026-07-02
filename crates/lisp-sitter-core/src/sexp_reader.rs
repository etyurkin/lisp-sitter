//! Shared byte-level s-expression scanner used by both validation and form-extraction code.
//!
//! The functions here operate on raw `&[u8]` and return byte positions.
//! They understand enough of Lisp syntax to correctly skip:
//!   - parenthesized lists `( ... )`
//!   - strings `" ... "` with `\` escape
//!   - line comments `;` to end-of-line
//!   - block comments `#| ... |#` (non‑nesting)
//!   - sexp comments `#;` (skip next form)
//!   - vectors `#( ... )`
//!   - character literals `#\a` (`#\(`) and elisp `?\(`
//!   - quote/quasiquote/unquote `'`, `` ` ``, `,`, `,@`
//!   - atoms (everything else until whitespace or delimiter)

pub type ScanResult<T> = std::result::Result<T, (usize, &'static str)>;

/// Which dialect's character-literal syntax the scanner should honor.
///
/// `#\(` / `#\)` (Common Lisp & Scheme) are handled for every dialect, since
/// `#\` never begins a token in Emacs Lisp. The elisp `?(` / `?\(` form is only
/// recognized under [`Dialect::Elisp`] so that a bare `?` symbol adjacent to a
/// paren in CL/Scheme is never mis-absorbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Generic,
    Elisp,
}

/// Byte length of the UTF-8 character whose leading byte is `b`.
fn utf8_len(b: u8) -> usize {
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

/// Advance past any whitespace and block comments starting at `pos`.
/// Returns the first non‑whitespace, non‑block-comment position.
pub fn skip_whitespace_and_comments(bytes: &[u8], mut pos: usize) -> usize {
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos + 1 < bytes.len() && bytes[pos] == b'#' && bytes[pos + 1] == b'|' {
            match skip_block_comment(bytes, pos) {
                Ok(next) => {
                    pos = next;
                    continue;
                }
                Err(_) => return pos,
            }
        }
        break;
    }
    pos
}

/// Skip one complete s‑expression starting at `pos` (dialect-agnostic / generic).
/// Returns the position immediately after it (or an error).
pub fn skip_sexp(bytes: &[u8], pos: usize) -> ScanResult<usize> {
    skip_sexp_in(bytes, pos, Dialect::Generic)
}

/// Skip one complete s‑expression, honoring `dialect`'s char-literal syntax.
pub fn skip_sexp_in(bytes: &[u8], pos: usize, dialect: Dialect) -> ScanResult<usize> {
    let mut i = pos;
    if i >= bytes.len() {
        return Err((i, "unexpected end of file"));
    }
    match bytes[i] {
        b'(' => {
            i += 1;
            loop {
                i = skip_whitespace_and_comments(bytes, i);
                if i >= bytes.len() {
                    return Err((i, "unbalanced parentheses"));
                }
                if bytes[i] == b')' {
                    return Ok(i + 1);
                }
                // #; sexp comment inside the list — skip the next form
                if i + 1 < bytes.len() && bytes[i] == b'#' && bytes[i + 1] == b';' {
                    i += 2;
                    i = skip_sexp_in(bytes, i, dialect)?;
                    continue;
                }
                i = skip_sexp_in(bytes, i, dialect)?;
            }
        }
        b'"' => skip_string(bytes, i),
        b';' => skip_line_comment(bytes, i),
        b'\'' | b'`' => skip_sexp_in(bytes, i + 1, dialect),
        b',' => {
            // Unquote or unquote-splicing
            let next = if i + 1 < bytes.len() && bytes[i + 1] == b'@' {
                i + 2
            } else {
                i + 1
            };
            skip_sexp_in(bytes, next, dialect)
        }
        b'#' => {
            if i + 1 < bytes.len() && bytes[i + 1] == b';' {
                // #; at top level — skip the following sexp
                skip_sexp_in(bytes, i + 2, dialect)
            } else {
                skip_atom_in(bytes, i, dialect)
            }
        }
        b')' => Err((i, "unmatched close paren")),
        _ => skip_atom_in(bytes, i, dialect),
    }
}

/// Skip a string literal `"..."` starting at `start` (the opening `"`).
pub fn skip_string(bytes: &[u8], start: usize) -> ScanResult<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            return Ok(i + 1);
        }
        i += 1;
    }
    Err((start, "unterminated string"))
}

/// Skip a line comment starting at `start` (the `;`).
pub fn skip_line_comment(bytes: &[u8], start: usize) -> ScanResult<usize> {
    let mut i = start + 1;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    Ok(i)
}

/// Skip a block comment `#| ... |#` starting at `start` (the `#` of `#|`).
///
/// Nesting is **not** handled (uncommon and complex for a fallback scanner).
pub fn skip_block_comment(bytes: &[u8], start: usize) -> ScanResult<usize> {
    let mut i = start + 2; // past '#|'
    while i + 1 < bytes.len() {
        if bytes[i] == b'|' && bytes[i + 1] == b'#' {
            return Ok(i + 2);
        }
        i += 1;
    }
    Err((start, "unterminated block comment"))
}

/// Skip an atom (symbol, number, `#t` / `#f`, `#\c`, `#(...)`, etc.).
///
/// Also handles the `#(...)` vector syntax by recursively scanning its contents
/// (vectors can contain nested sexps and `#;` comments).
pub fn skip_atom(bytes: &[u8], start: usize) -> ScanResult<usize> {
    skip_atom_in(bytes, start, Dialect::Generic)
}

/// Skip an atom, honoring `dialect`'s character-literal syntax.
pub fn skip_atom_in(bytes: &[u8], start: usize, dialect: Dialect) -> ScanResult<usize> {
    let mut i = start;
    if i >= bytes.len() {
        return Err((i, "unexpected end of file"));
    }
    // #( ... ) vector syntax
    if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
        i += 2;
        loop {
            i = skip_whitespace_and_comments(bytes, i);
            if i >= bytes.len() {
                return Err((i, "unterminated vector"));
            }
            if bytes[i] == b')' {
                return Ok(i + 1);
            }
            if i + 1 < bytes.len() && bytes[i] == b'#' && bytes[i + 1] == b';' {
                i += 2;
                i = skip_sexp_in(bytes, i, dialect)?;
                continue;
            }
            i = skip_sexp_in(bytes, i, dialect)?;
        }
    }
    // #\c character literal (Common Lisp / Scheme): the char after #\ is taken
    // verbatim, even if it is `(`, `)`, `"`, `;` or whitespace (e.g. #\( #\Space).
    if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
        i += 2;
        if i < bytes.len() {
            i += utf8_len(bytes[i]);
        }
        // Fall through to the normal atom scan to pick up named chars (#\Space).
    }
    // ?c / ?\c character literal (Emacs Lisp): the char after ? (or ?\) is taken
    // verbatim, even if it is `(` or `)` (e.g. ?\( ?\) ?( ).
    else if dialect == Dialect::Elisp && bytes[i] == b'?' {
        i += 1;
        if i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
        }
        if i < bytes.len() {
            i += utf8_len(bytes[i]);
        }
        // Fall through to the normal atom scan to pick up modifiers (?\C-x).
    }
    // Regular atom — consume until whitespace or delimiter
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !is_delim(bytes[i]) {
        // Handle |...| escaped symbols in Common Lisp
        if bytes[i] == b'|' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'|' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // skip closing |
            }
            continue;
        }
        i += 1;
    }
    Ok(i)
}

/// True if `b` is a Lisp delimiter that ends an atom.
pub fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'"' | b';')
}

/// Given a possibly-incomplete s-expression, append enough `)` characters to
/// close all open parens.  Returns `None` if the input has a stray `)` that
/// can't be fixed by appending.
///
/// Strings, comments, `#;` sexp comments, `#|...|#` block comments, vectors
/// `#(...)`, quotes `'` / `` ` ``, unquote `,` / `,@`, and pipe-escaped symbols
/// `|...|` are all handled correctly.
pub fn complete_form(input: &str) -> Option<String> {
    complete_form_in(input, Dialect::Generic)
}

/// Like [`complete_form`], honoring `dialect`'s char-literal syntax so that an
/// open/close paren inside a `?\(` / `#\(` literal is not counted toward depth.
pub fn complete_form_in(input: &str, dialect: Dialect) -> Option<String> {
    let bytes = input.as_bytes();
    let mut depth: u32 = 0;
    let mut i = 0;

    while i < bytes.len() {
        // Collapse whitespace + block comments
        let before = i;
        i = skip_whitespace_and_comments(bytes, i);
        if before != i {
            continue;
        }

        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                if depth == 0 {
                    return None; // unmatched close — can't fix
                }
                depth -= 1;
                i += 1;
            }
            b'"' => {
                if let Ok(next) = skip_string(bytes, i) {
                    i = next;
                } else {
                    // Unterminated string — stop counting here
                    break;
                }
            }
            b';' => {
                if let Ok(next) = skip_line_comment(bytes, i) {
                    i = next;
                } else {
                    i += 1;
                }
            }
            // #\c (CL/Scheme) or ?\c / ?c (elisp) char literal — consume as one
            // atom so an embedded paren isn't counted.
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b'\\' => {
                match skip_atom_in(bytes, i, dialect) {
                    Ok(next) => i = next,
                    Err(_) => i += 1,
                }
            }
            b'?' if dialect == Dialect::Elisp => match skip_atom_in(bytes, i, dialect) {
                Ok(next) => i = next,
                Err(_) => i += 1,
            },
            b'\'' | b'`' => i += 1,
            b',' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'@' {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            b'#' => {
                if i + 1 < bytes.len() {
                    if bytes[i + 1] == b';' {
                        // #; — skip the next form
                        if let Ok(next) = skip_sexp_in(bytes, i + 2, dialect) {
                            i = next;
                            continue;
                        }
                        // Next form is incomplete — stop here
                        break;
                    }
                    if bytes[i + 1] == b'|' {
                        // Block comment already handled by skip_whitespace
                        i += 1;
                        continue;
                    }
                }
                i += 1;
            }
            // Pipe-escaped symbol |...| is handled by skip_atom
            b'|' => {
                if let Ok(next) = skip_atom(bytes, i) {
                    i = next;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }

    if depth == 0 {
        // Already balanced
        Some(input.to_string())
    } else {
        let mut result = input.to_string();
        for _ in 0..depth {
            result.push(')');
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok<const N: usize>(src: &[u8; N], start: usize) -> usize {
        skip_sexp(src, start).unwrap()
    }

    fn err(src: &[u8], start: usize) -> &'static str {
        skip_sexp(src, start).unwrap_err().1
    }

    #[test]
    fn simple_list() {
        assert_eq!(ok(b"(defun x () 1)", 0), 14);
    }

    #[test]
    fn empty_list() {
        assert_eq!(ok(b"()", 0), 2);
    }

    #[test]
    fn string_with_escape() {
        // "hello\"world" — the \" is an escaped quote inside the string
        assert_eq!(ok(b"\"hello\\\"world\"", 0), 14);
    }

    #[test]
    fn list_with_string() {
        assert_eq!(ok(b"(print \"hi\")", 0), 12);
    }

    #[test]
    fn unbalanced() {
        assert_eq!(err(b"(defun x (", 0), "unbalanced parentheses");
    }

    #[test]
    fn unbalanced_string() {
        assert_eq!(err(b"\"hello", 0), "unterminated string");
    }

    #[test]
    fn unmatched_close() {
        assert_eq!(err(b")", 0), "unmatched close paren");
    }

    #[test]
    fn quote() {
        assert_eq!(ok(b"'foo", 0), 4);
    }

    #[test]
    fn quoted_list() {
        assert_eq!(ok(b"'(1 2 3)", 0), 8);
    }

    #[test]
    fn sexp_comment() {
        // skip #;(dead) → past the closing ), stop at 'l'
        assert_eq!(ok(b"#;(dead)live", 0), 8);
    }

    #[test]
    fn sexp_comment_inside_list() {
        assert_eq!(ok(b"(foo #;(dead) bar)", 0), 18);
    }

    #[test]
    fn block_comment() {
        let src = b"(foo #| comment |# bar)";
        assert_eq!(skip_sexp(src, 0).unwrap(), 23);
    }

    #[test]
    fn block_comment_top_level() {
        //       #| dead |# (live)
        //  i=0         10        17
        let src = b"#| dead |# (live)";
        let i = skip_whitespace_and_comments(src, 0);
        assert_eq!(i, 11);
        assert_eq!(skip_sexp(src, i).unwrap(), 17);
    }

    #[test]
    fn vector() {
        assert_eq!(ok(b"#(1 2 3)", 0), 8);
    }

    #[test]
    fn vector_with_comment() {
        assert_eq!(ok(b"#(1 #;dead 3)", 0), 13);
    }

    #[test]
    fn comma_at() {
        let src = b",@foo";
        assert_eq!(ok(src, 0), 5);
    }

    #[test]
    fn pipe_escaped_symbol() {
        let src = b"|some symbol|";
        assert_eq!(ok(src, 0), 13);
    }

    #[test]
    fn shebang_comment_is_just_an_atom() {
        let src = b"#!/usr/bin/scheme --script\n(define x 1)";
        let end = skip_sexp(src, 0).unwrap();
        assert!(end > 0);
    }

    // ── complete_form tests ──

    #[test]
    fn complete_balanced() {
        assert_eq!(
            complete_form("(defun x () 1)").as_deref(),
            Some("(defun x () 1)")
        );
    }

    #[test]
    fn complete_missing_one() {
        assert_eq!(
            complete_form("(defun x () 1").as_deref(),
            Some("(defun x () 1)")
        );
    }

    #[test]
    fn complete_missing_two() {
        assert_eq!(
            complete_form("(defun x (if y 1").as_deref(),
            Some("(defun x (if y 1))")
        );
    }

    #[test]
    fn complete_unmatched_close() {
        assert_eq!(complete_form("(defun x () 1))"), None);
    }

    #[test]
    fn complete_missing_string() {
        assert_eq!(
            complete_form("(defun x () \"hello").as_deref(),
            Some("(defun x () \"hello)")
        );
    }

    #[test]
    fn complete_paren_in_string() {
        assert_eq!(
            complete_form("(defun x () \"(hello\" 1").as_deref(),
            Some("(defun x () \"(hello\" 1)")
        );
    }

    #[test]
    fn complete_skips_comment() {
        assert_eq!(
            complete_form("(defun x (if y 1) ; (comment\n").as_deref(),
            Some("(defun x (if y 1) ; (comment\n)")
        );
    }

    #[test]
    fn complete_sexp_comment() {
        assert_eq!(
            complete_form("#;(dead) (define x 1").as_deref(),
            Some("#;(dead) (define x 1)")
        );
    }

    #[test]
    fn complete_pipe_escaped() {
        assert_eq!(
            complete_form("(|something| x").as_deref(),
            Some("(|something| x)")
        );
    }

    #[test]
    fn complete_vector() {
        assert_eq!(complete_form("#(foo bar").as_deref(), Some("#(foo bar)"));
    }

    #[test]
    fn complete_empty_is_balanced() {
        assert_eq!(complete_form("").as_deref(), Some(""));
    }
}
