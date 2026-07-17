//! Character-level scanners that understand the *shape of a definition form* —
//! head, name, parameter list, docstring/`declare` preamble, and body — built
//! on the primitives in [`crate::sexp_reader`].
//!
//! These are the tree-sitter-free fallbacks the transform layer uses when a
//! grammar isn't available, and small structural helpers (element splitting,
//! enclosing-form lookup, whole-symbol substitution). Keeping them here beside
//! the byte scanner means all "how to parse a form" knowledge lives in one
//! place rather than being split across crates.

use crate::error::Error;
use crate::sexp_reader::{
    skip_atom_in, skip_block_comment, skip_line_comment, skip_sexp_in, skip_string,
    skip_whitespace_and_comments, Dialect,
};

/// Advance past ASCII whitespace.
pub fn skip_sp(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Advance past a bare symbol token (stops at whitespace or a paren). Whitespace
/// and parens are ASCII, so this never splits a multi-byte character.
pub fn skip_sym(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'(' && bytes[i] != b')'
    {
        i += 1;
    }
    i
}

/// Skip one s-expression with the generic scanner, leaving `i` unchanged on
/// malformed input so loop callers can guard against no-progress.
pub fn skip_sexp(bytes: &[u8], i: usize) -> usize {
    skip_sexp_d(bytes, i, Dialect::Generic)
}

/// Skip one s-expression honoring `d`'s char literals.
pub fn skip_sexp_d(bytes: &[u8], i: usize, d: Dialect) -> usize {
    skip_sexp_in(bytes, i, d).unwrap_or(i)
}

/// Find the first occurrence of `pat` as a complete syntactic sub-expression in
/// `ft`, skipping strings, comments, and char literals. Character-level fallback
/// used when tree-sitter is unavailable.
pub fn find_sexp_char(ft: &str, pat: &str, d: Dialect) -> Option<(usize, usize)> {
    let b = ft.as_bytes();
    let pb = pat.as_bytes();
    let mut i = 0;
    while i < b.len() {
        i = match b[i] {
            b'"' => skip_string(b, i).unwrap_or(b.len()),
            b';' => skip_line_comment(b, i).unwrap_or(b.len()),
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => {
                skip_block_comment(b, i).unwrap_or(b.len())
            }
            b'?' if d == Dialect::Elisp => skip_sexp_in(b, i, d).unwrap_or(i + 1),
            _ => {
                if i + pb.len() <= b.len() && &b[i..i + pb.len()] == pb {
                    let n = i + pb.len();
                    let po = i == 0 || b[i - 1].is_ascii_whitespace() || b[i - 1] == b'(';
                    let no =
                        n >= b.len() || b[n].is_ascii_whitespace() || b[n] == b')' || b[n] == b'(';
                    if po && no {
                        return Some((i, n));
                    }
                }
                i + 1
            }
        };
    }
    None
}

/// Skip leading docstrings and Common Lisp `(declare …)` forms so a body range
/// starts at the first executable form.
pub fn skip_preamble(b: &[u8], mut pos: usize) -> usize {
    loop {
        pos = skip_sp(b, pos);
        if pos >= b.len() {
            break;
        }
        if b[pos] == b'"' {
            pos = skip_sexp(b, pos);
        } else if b[pos] == b'(' {
            let inner = skip_sp(b, pos + 1);
            let kw_end = skip_sym(b, inner);
            if kw_end > inner && &b[inner..kw_end] == b"declare" {
                pos = skip_sexp(b, pos);
            } else {
                break;
            }
        } else {
            break;
        }
    }
    pos
}

/// The body byte range of a definition form, skipping head, name, param list,
/// and preamble. Character-level fallback for the tree-sitter body analysis.
pub fn body_range_char(ft: &str) -> Result<(usize, usize), Error> {
    let b = ft.as_bytes();
    let mut pos = 0;
    if pos >= b.len() || b[pos] != b'(' {
        return Err(Error::InvalidArgs("form must start with (".into()));
    }
    pos += 1;
    pos = skip_sp(b, pos);
    pos = skip_sym(b, pos);
    pos = skip_sp(b, pos);
    pos = skip_sexp(b, pos);
    pos = skip_sp(b, pos);
    pos = skip_sexp(b, pos);
    pos = skip_sp(b, pos);
    pos = skip_preamble(b, pos);
    let bs = pos;
    if b.last() != Some(&b')') {
        return Err(Error::InvalidArgs("form must end with )".into()));
    }
    let be = ft.len() - 1;
    if bs > be {
        return Err(Error::InvalidArgs("no body to wrap".into()));
    }
    Ok((bs, be))
}

/// Rename the definition-name token inside a form's text (handles both
/// `(head name …)` and Scheme curried `(head (name …) …)`). Character-level
/// fallback for the tree-sitter rename.
pub fn replace_name_in_form_char(t: &str, old: &str, new: &str) -> String {
    let s = t.trim();
    if !s.starts_with('(') {
        return t.to_string();
    }
    let a = &s[1..].trim_start();
    let he = a.find(|c: char| c.is_whitespace()).unwrap_or(0);
    if he == 0 {
        return t.to_string();
    }
    let ah = &a[he..].trim_start();
    if let Some(inner) = ah.strip_prefix('(') {
        let ne = inner
            .find(|c: char| c.is_whitespace() || c == ')')
            .unwrap_or(inner.len());
        if &inner[..ne] == old {
            return format!("({} ({}{}", &a[..he], new, &inner[ne..]);
        }
    } else {
        let ne = ah.find(|c: char| c.is_whitespace()).unwrap_or(ah.len());
        if &ah[..ne] == old {
            return format!("({} {}{}", &a[..he], new, &ah[ne..]);
        }
    }
    t.to_string()
}

/// Byte ranges of the immediate child elements inside the outer list of `ft`.
pub fn split_elements(ft: &str, d: Dialect) -> Vec<(usize, usize)> {
    let b = ft.as_bytes();
    let open = match ft.find('(') {
        Some(o) => o + 1,
        None => return Vec::new(),
    };
    let close = ft.rfind(')').unwrap_or(ft.len());
    let mut i = open;
    let mut elems = Vec::new();
    while i < close {
        i = skip_sp(b, i);
        if i >= close {
            break;
        }
        let s = i;
        let e = skip_sexp_d(b, i, d).min(close);
        if e <= s {
            break;
        }
        elems.push((s, e));
        i = e;
    }
    elems
}

/// Wrap multiple body forms into one expression if needed.
/// `seq_kw` is the dialect sequencing form (`progn` or `begin`).
pub fn wrap_multi_body(body_text: &str, d: Dialect, seq_kw: &str) -> String {
    let b = body_text.as_bytes();
    let mut i = 0;
    let mut count = 0;
    while i < b.len() {
        i = skip_whitespace_and_comments(b, i);
        if i >= b.len() {
            break;
        }
        match skip_sexp_in(b, i, d) {
            Ok(next) => {
                count += 1;
                i = next;
            }
            Err(_) => break,
        }
    }
    if count <= 1 {
        body_text.trim().to_string()
    } else {
        format!("({seq_kw} {})", body_text.trim())
    }
}

/// Extract `(param names, single body expression)` from a definition form.
/// Character-level fallback for the tree-sitter analysis.
pub fn def_params_and_body_char(ft: &str, d: Dialect) -> Option<(Vec<String>, String)> {
    let elems = split_elements(ft, d);
    if elems.len() < 3 {
        return None;
    }
    let head = &ft[elems[0].0..elems[0].1];
    let close = ft.rfind(')').unwrap_or(ft.len());

    let (params, body_idx) = if head == "define" && ft[elems[1].0..elems[1].1].starts_with('(') {
        // Scheme curried define: (define (name p…) body…)
        let sig = &ft[elems[1].0..elems[1].1];
        let sig_elems = split_elements(sig, d);
        if sig_elems.is_empty() {
            return None;
        }
        let params = sig_elems
            .iter()
            .skip(1)
            .map(|(s, e)| sig[*s..*e].to_string())
            .collect();
        (params, 2)
    } else {
        // (head name (args) body…)
        let arglist = &ft[elems[2].0..elems[2].1];
        if !arglist.starts_with('(') {
            return None;
        }
        let params = split_elements(arglist, d)
            .iter()
            .map(|(s, e)| arglist[*s..*e].to_string())
            .collect();
        (params, 3)
    };

    if elems.len() <= body_idx {
        return None;
    }
    let body_text = ft[elems[body_idx].0..close].trim().to_string();
    if body_text.is_empty() {
        return None;
    }
    let body = if elems.len() - body_idx > 1 {
        let kw = if d == Dialect::Generic && head == "define" {
            "begin"
        } else {
            "progn"
        };
        format!("({kw} {body_text})")
    } else {
        body_text
    };
    Some((params, body))
}

/// Replace every whole-symbol occurrence of `name` with `repl`, skipping
/// strings, comments, and char literals.
pub fn substitute_symbol(text: &str, name: &str, repl: &str, d: Dialect) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let e = skip_string(b, i).unwrap_or(b.len());
                out.push_str(&text[i..e]);
                i = e;
            }
            b';' => {
                let e = skip_line_comment(b, i).unwrap_or(b.len());
                out.push_str(&text[i..e]);
                i = e;
            }
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => {
                let e = skip_block_comment(b, i).unwrap_or(b.len());
                out.push_str(&text[i..e]);
                i = e;
            }
            b'#' if i + 1 < b.len() && b[i + 1] == b'\\' => {
                let e = skip_sexp_d(b, i, d);
                out.push_str(&text[i..e]);
                i = e;
            }
            b'?' if d == Dialect::Elisp => {
                let e = skip_sexp_d(b, i, d);
                out.push_str(&text[i..e]);
                i = e;
            }
            b'(' | b')' | b'\'' | b'`' | b',' => {
                out.push(b[i] as char);
                i += 1;
            }
            c if c.is_ascii_whitespace() => {
                out.push(c as char);
                i += 1;
            }
            _ => {
                let s = i;
                let e = skip_sym(b, i).max(s + 1);
                let tok = &text[s..e];
                if tok == name {
                    out.push_str(repl);
                } else {
                    out.push_str(tok);
                }
                i = e;
            }
        }
    }
    out
}

/// The byte range of the innermost list that encloses `inner_start`, or `None`
/// if `inner_start` is at top level. Skips strings, comments, and char literals.
pub fn find_enclosing_sexp(text: &str, inner_start: usize, d: Dialect) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < inner_start {
        while i < inner_start && i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= inner_start || i >= b.len() {
            break;
        }
        match b[i] {
            b'(' => {
                stack.push(i);
                i += 1;
            }
            b')' => {
                stack.pop();
                i += 1;
            }
            b'"' => {
                i = skip_string(b, i).unwrap_or(b.len());
            }
            b';' => {
                i = skip_line_comment(b, i).unwrap_or(b.len());
            }
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => {
                i = skip_block_comment(b, i).unwrap_or(b.len());
            }
            b'#' if i + 1 < b.len() && b[i + 1] == b';' => {
                i = skip_sexp_in(b, i + 2, d).unwrap_or(b.len());
            }
            b'\'' | b'`' => {
                i += 1;
            }
            b',' => {
                i += if i + 1 < b.len() && b[i + 1] == b'@' {
                    2
                } else {
                    1
                };
            }
            _ => {
                i = skip_atom_in(b, i, d).unwrap_or(i + 1);
            }
        }
    }
    let parent_start = *stack.last()?;
    let parent_end = skip_sexp_in(b, parent_start, d).ok()?;
    Some((parent_start, parent_end))
}

/// Count the top-level forms in `body`.
pub fn count_forms(body: &str) -> usize {
    let b = body.as_bytes();
    let mut i = skip_sp(b, 0);
    let mut count = 0;
    while i < b.len() {
        let n = skip_sexp(b, i);
        if n <= i {
            break;
        }
        count += 1;
        i = skip_sp(b, n);
    }
    count
}
