//! S-expression call-site scanner shared by semantic analysis and the call graph.

use lisp_sitter_core::sexp_reader::{skip_block_comment, skip_line_comment, skip_sexp_in};
use lisp_sitter_core::Dialect;

#[derive(Debug, Clone)]
pub struct Call {
    pub name: String,
    pub pos: usize,
    pub argc: usize,
}

/// Collect every `(head arg…)` call list in `content`, recording the head
/// symbol, the byte offset of the opening paren, and the argument count.
pub fn scan_calls(content: &str, dialect: Dialect) -> Vec<Call> {
    let mut out = Vec::new();
    scan_range(content, content.as_bytes(), 0, content.len(), dialect, &mut out);
    out
}

fn scan_range(content: &str, b: &[u8], mut i: usize, end: usize, d: Dialect, out: &mut Vec<Call>) {
    while i < end {
        i = skip_ws_comments(b, i, end);
        if i >= end {
            break;
        }
        if b[i] == b')' {
            i += 1;
            continue;
        }
        if b[i] == b'(' {
            let close = skip_sexp_in(b, i, d).unwrap_or(end).min(end);
            scan_list(content, b, i, close, d, out);
            i = if close > i { close } else { i + 1 };
        } else {
            let n = skip_sexp_in(b, i, d).unwrap_or(i + 1);
            i = if n > i { n } else { i + 1 };
        }
    }
}

/// Byte ranges of the immediate child forms inside the list `[open, close)`.
pub fn list_children(b: &[u8], open: usize, close: usize, d: Dialect) -> Vec<(usize, usize)> {
    let mut kids = Vec::new();
    let mut k = skip_ws_comments(b, open + 1, close);
    while k < close && b[k] != b')' {
        let n = skip_sexp_in(b, k, d).unwrap_or(close).min(close);
        let n = if n > k { n } else { k + 1 };
        kids.push((k, n));
        k = skip_ws_comments(b, n, close);
    }
    kids
}

fn scan_list(content: &str, b: &[u8], open: usize, close: usize, d: Dialect, out: &mut Vec<Call>) {
    let kids = list_children(b, open, close, d);
    let Some(&(hs, he)) = kids.first() else { return };

    let head = (!matches!(b[hs], b'(' | b'"' | b'\'' | b'`' | b',' | b'#')).then(|| &content[hs..he]);
    if let Some(h) = head {
        if is_call_name(h) {
            out.push(Call { name: h.to_string(), pos: open, argc: kids.len() - 1 });
        }
    }

    let recurse = |ranges: &[(usize, usize)], out: &mut Vec<Call>| {
        for (s, e) in ranges {
            scan_range(content, b, *s, *e, d, out);
        }
    };

    match head {
        Some(h) if is_fn_def_head(h) => recurse(kids.get(3..).unwrap_or(&[]), out),
        Some("lambda") => recurse(kids.get(2..).unwrap_or(&[]), out),
        Some("define") => recurse(kids.get(2..).unwrap_or(&[]), out),
        Some(h) if is_let_head(h) => {
            if let Some(&(bs, be)) = kids.get(1) {
                if b[bs] == b'(' {
                    for (vs, ve) in list_children(b, bs, be, d) {
                        if b[vs] == b'(' {
                            recurse(list_children(b, vs, ve, d).get(1..).unwrap_or(&[]), out);
                        }
                    }
                }
            }
            recurse(kids.get(2..).unwrap_or(&[]), out);
        }
        _ => recurse(&kids, out),
    }
}

fn is_fn_def_head(h: &str) -> bool {
    matches!(h, "defun" | "defsubst" | "defmacro" | "cl-defun" | "cl-defmacro")
}

fn is_let_head(h: &str) -> bool {
    matches!(h, "let" | "let*" | "letrec" | "letrec*" | "when-let" | "if-let" | "cl-flet" | "cl-labels")
}

pub fn skip_ws_comments(b: &[u8], mut i: usize, end: usize) -> usize {
    loop {
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < end && b[i] == b';' {
            i = skip_line_comment(b, i).unwrap_or(end).min(end);
            continue;
        }
        if i + 1 < end && b[i] == b'#' && b[i + 1] == b'|' {
            i = skip_block_comment(b, i).unwrap_or(end).min(end);
            continue;
        }
        break;
    }
    i
}

pub fn is_call_name(s: &str) -> bool {
    let Some(c0) = s.chars().next() else { return false };
    if matches!(c0, ':' | '#' | '"') || c0.is_ascii_digit() {
        return false;
    }
    !matches!(s, "." | "t" | "nil" | "else")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_counts_args_and_skips_quotes() {
        let calls = scan_calls("(foo a b (bar c) '(not a call))", Dialect::Elisp);
        let foo = calls.iter().find(|c| c.name == "foo").unwrap();
        assert_eq!(foo.argc, 4);
        let bar = calls.iter().find(|c| c.name == "bar").unwrap();
        assert_eq!(bar.argc, 1);
        assert!(!calls.iter().any(|c| c.name == "not"));
    }
}
