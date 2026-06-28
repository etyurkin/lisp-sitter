//! Scan top-level s-expressions and extract definer/name pairs for the fallback
//! path used when the tree-sitter parse is unavailable or empty.
//!
//! Form recognition and naming are delegated to [`crate::definers::DefinerSet`]
//! so this fallback recognizes exactly the same forms as the tree-sitter path.

use crate::definers::DefinerSet;
use crate::position::error_at;
use crate::sexp_reader::{self, skip_whitespace_and_comments, Dialect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedForm {
    pub start: usize,
    pub end: usize,
    pub head: String,
    pub name: String,
}

pub fn top_level_definer_forms(
    content: &str,
    set: &DefinerSet,
    dialect: Dialect,
) -> Result<Vec<ScannedForm>, String> {
    let bytes = content.as_bytes();
    let mut pos = 0usize;
    let mut forms = Vec::new();
    loop {
        pos = skip_whitespace_and_comments(bytes, pos);
        if pos >= bytes.len() {
            break;
        }
        let start = pos;
        let end = sexp_reader::skip_sexp_in(bytes, pos, dialect)
            .map_err(|(p, m)| error_at(content, p, m))?;
        if let Some((head, name)) = set.classify(&content[start..end]) {
            forms.push(ScannedForm { start, end, head, name });
        }
        pos = end;
    }
    Ok(forms)
}

pub fn find_form_bounds(
    content: &str,
    symbol: &str,
    set: &DefinerSet,
    dialect: Dialect,
) -> Option<(usize, usize)> {
    top_level_definer_forms(content, set, dialect)
        .ok()?
        .into_iter()
        .find(|f| f.name == symbol)
        .map(|f| (f.start, f.end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definers::Definer;

    fn defun_set() -> DefinerSet {
        DefinerSet::new(vec![Definer::second("defun")])
    }

    #[test]
    fn scans_defun_forms() {
        let content = "(defun alpha () 1)\n\n(defun beta () 2)\n";
        let forms = top_level_definer_forms(content, &defun_set(), Dialect::Generic).unwrap();
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].name, "alpha");
        assert_eq!(forms[1].name, "beta");
    }

    #[test]
    fn finds_bounds_by_name() {
        let content = "(defun alpha () 1)\n(defun beta () 2)\n";
        let b = find_form_bounds(content, "beta", &defun_set(), Dialect::Generic).unwrap();
        assert!(b.0 < b.1);
    }

    #[test]
    fn block_comment_skipped() {
        let content = "#| dead |#\n(defun alive () 1)\n";
        let forms = top_level_definer_forms(content, &defun_set(), Dialect::Generic).unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name, "alive");
    }

    #[test]
    fn sexp_comment_skipped() {
        let content = "#;(defun dead ())\n(defun alive () 1)\n";
        let forms = top_level_definer_forms(content, &defun_set(), Dialect::Generic).unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name, "alive");
    }
}
