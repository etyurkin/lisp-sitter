use crate::sexp_reader::{self, skip_whitespace_and_comments, Dialect};

pub fn content_blank(content: &str) -> bool {
    content.trim().is_empty()
}

pub fn replace_region(content: &str, start: usize, end: usize, replacement: &str) -> String {
    // Snap offsets to UTF-8 char boundaries so slicing never panics on a bad
    // offset (e.g. one landing inside a multibyte character).
    let start = floor_char_boundary(content, start);
    let end = ceil_char_boundary(content, end.max(start));
    let mut out = String::with_capacity(content.len() + replacement.len());
    out.push_str(&content[..start]);
    out.push_str(replacement);
    if end < content.len() {
        out.push_str(&content[end..]);
    }
    out
}

/// Largest char boundary `<= i` (clamped to `s.len()`).
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary `>= i` (clamped to `s.len()`).
fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub fn scan_parens(content: &str) -> Option<String> {
    scan_parens_in(content, Dialect::Generic)
}

pub fn scan_parens_in(content: &str, dialect: Dialect) -> Option<String> {
    let bytes = content.as_bytes();
    let mut i = 0usize;
    loop {
        i = skip_whitespace_and_comments(bytes, i);
        if i >= bytes.len() {
            break;
        }
        match sexp_reader::skip_sexp_in(bytes, i, dialect) {
            Ok(next) => i = next,
            Err((pos, msg)) => return Some(crate::position::error_at(content, pos, msg)),
        }
    }
    i = skip_whitespace_and_comments(bytes, i);
    if i < bytes.len() {
        return Some(crate::position::error_at(
            content,
            i,
            "extra text after last form",
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_parens_ok() {
        assert!(scan_parens("(defun x () 1)\n").is_none());
    }

    #[test]
    fn unbalanced_reports_error() {
        assert!(scan_parens("(defun x () 1").is_some());
    }

    #[test]
    fn sexp_comment_passes() {
        assert!(scan_parens("(defun x () #;(dead) 1)\n").is_none());
    }

    #[test]
    fn block_comment_passes() {
        assert!(scan_parens("#| header |#\n(defun x () 1)\n").is_none());
    }
}
