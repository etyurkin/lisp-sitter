use crate::sexp_reader::{self, skip_whitespace_and_comments};

pub fn content_blank(content: &str) -> bool {
    content.trim().is_empty()
}

pub fn replace_region(content: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut out = String::with_capacity(content.len() + replacement.len());
    out.push_str(&content[..start.min(content.len())]);
    out.push_str(replacement);
    if end < content.len() {
        out.push_str(&content[end..]);
    }
    out
}

pub fn scan_parens(content: &str) -> Option<String> {
    let bytes = content.as_bytes();
    let mut i = 0usize;
    loop {
        i = skip_whitespace_and_comments(bytes, i);
        if i >= bytes.len() {
            break;
        }
        match sexp_reader::skip_sexp(bytes, i) {
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
