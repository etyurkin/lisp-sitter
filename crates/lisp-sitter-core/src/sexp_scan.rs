/// Scan top-level s-expressions and extract definer/name pairs for fallback bounds.
use crate::position::error_at;
use crate::sexp_reader::{self, skip_whitespace_and_comments};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedForm {
    pub start: usize,
    pub end: usize,
    pub head: String,
    pub name: String,
}

pub fn top_level_definer_forms(content: &str, definers: &[&str]) -> Result<Vec<ScannedForm>, String> {
    let bytes = content.as_bytes();
    let mut pos = 0usize;
    let mut forms = Vec::new();
    loop {
        pos = skip_whitespace_and_comments(bytes, pos);
        if pos >= bytes.len() {
            break;
        }
        let start = pos;
        let end = sexp_reader::skip_sexp(bytes, pos).map_err(|(p, m)| error_at(content, p, m))?;
        if let Some(form) = parse_definer_form(&content[start..end], definers) {
            forms.push(ScannedForm {
                start,
                end,
                head: form.0,
                name: form.1,
            });
        }
        pos = end;
    }
    Ok(forms)
}

pub fn find_form_bounds(
    content: &str,
    symbol: &str,
    definers: &[&str],
) -> Option<(usize, usize)> {
    top_level_definer_forms(content, definers)
        .ok()?
        .into_iter()
        .find(|f| f.name == symbol)
        .map(|f| (f.start, f.end))
}

fn parse_definer_form(text: &str, definers: &[&str]) -> Option<(String, String)> {
    let trimmed = text.trim();
    if !trimmed.starts_with('(') {
        return None;
    }
    let inner = trimmed.strip_prefix('(')?.trim_start();
    let (head, rest) = read_atom(inner).ok()?;
    if !definers.iter().any(|d| head == *d) {
        return None;
    }
    let rest = rest.trim_start();
    let name = if rest.starts_with('(') {
        let list_inner = rest.strip_prefix('(')?.trim_start();
        let (sym, _) = read_atom(list_inner).ok()?;
        if head == "define-library" {
            collect_list_symbols(rest)
        } else {
            sym
        }
    } else {
        read_atom(rest).ok()?.0
    };
    if name.is_empty() {
        None
    } else {
        Some((head, name))
    }
}

fn collect_list_symbols(list_text: &str) -> String {
    let trimmed = list_text.trim();
    if !trimmed.starts_with('(') {
        return String::new();
    }
    let mut inner = trimmed.strip_prefix('(').unwrap().trim_start();
    let mut parts = Vec::new();
    while !inner.is_empty() && !inner.starts_with(')') {
        if let Ok((sym, rest)) = read_atom(inner) {
            if !sym.is_empty() {
                parts.push(sym);
            }
            inner = rest.trim_start();
        } else {
            break;
        }
    }
    parts.join(" ")
}

fn read_atom(input: &str) -> Result<(String, &str), ()> {
    let input = input.trim_start();
    if input.is_empty() {
        return Err(());
    }
    if input.starts_with('(') || input.starts_with(')') {
        return Err(());
    }
    if input.starts_with('"') {
        return Err(());
    }
    let end = input
        .find(|c: char| c.is_whitespace() || c == ')')
        .unwrap_or(input.len());
    let atom = input[..end].to_string();
    Ok((atom, &input[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_defun_forms() {
        let content = "(defun alpha () 1)\n\n(defun beta () 2)\n";
        let forms = top_level_definer_forms(content, &["defun"]).unwrap();
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].name, "alpha");
        assert_eq!(forms[1].name, "beta");
    }

    #[test]
    fn finds_bounds_by_name() {
        let content = "(defun alpha () 1)\n(defun beta () 2)\n";
        let b = find_form_bounds(content, "beta", &["defun"]).unwrap();
        assert!(b.0 < b.1);
    }

    #[test]
    fn block_comment_skipped() {
        let content = "#| dead |#\n(defun alive () 1)\n";
        let forms = top_level_definer_forms(content, &["defun"]).unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name, "alive");
    }

    #[test]
    fn sexp_comment_skipped() {
        let content = "#;(defun dead ())\n(defun alive () 1)\n";
        let forms = top_level_definer_forms(content, &["defun"]).unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name, "alive");
    }
}
