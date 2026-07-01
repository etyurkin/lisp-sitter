//! Declarative description of a dialect's top-level "definer" forms.
//!
//! A [`DefinerSet`] is the single source of truth for *what counts as a
//! top-level definition* and *how to read its name*. It is consumed by both
//! the tree-sitter path (classify the source text of each top-level node) and
//! the s-expression fallback (`sexp_scan`), so the two never drift apart.
//!
//! Naming is purely text-based (it does not depend on grammar node kinds),
//! which lets a single code path handle dedicated grammar nodes
//! (elisp `function_definition`, CL `defun`…) and plain `list` nodes alike.

/// How to extract the defined name from a form's argument list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameStrategy {
    /// The name is the second element. If it is a list, take its head symbol
    /// (handles Scheme's curried `(define (f x) …)` and CL `defmethod`); a
    /// leading `'`, `` ` `` or `:` sigil and surrounding `"…"` are stripped.
    Second,
    /// The name is the second element, a list whose symbols are joined with
    /// spaces (Scheme's `define-library (my lib)` → `my lib`).
    LibraryList,
}

/// One recognized definer keyword and its name-extraction strategy.
#[derive(Debug, Clone)]
pub struct Definer {
    pub keyword: String,
    pub name: NameStrategy,
}

impl Definer {
    /// A definer whose name is the second element (the common case).
    pub fn second(keyword: impl Into<String>) -> Self {
        Self { keyword: keyword.into(), name: NameStrategy::Second }
    }

    pub fn new(keyword: impl Into<String>, name: NameStrategy) -> Self {
        Self { keyword: keyword.into(), name }
    }
}

/// The set of definer forms for one dialect. Owned so a base set can be
/// extended at runtime with user-configured definers.
#[derive(Debug, Clone, Default)]
pub struct DefinerSet {
    defs: Vec<Definer>,
}

impl DefinerSet {
    pub fn new(defs: Vec<Definer>) -> Self {
        Self { defs }
    }

    /// Add extra keywords (with [`NameStrategy::Second`]) unless already present.
    pub fn extend_keywords(&mut self, keywords: &[String]) {
        for k in keywords {
            let k = k.trim();
            if !k.is_empty() && !self.contains(k) {
                self.defs.push(Definer::second(k));
            }
        }
    }

    /// True if `head` is a recognized definer keyword.
    pub fn contains(&self, head: &str) -> bool {
        self.defs.iter().any(|d| d.keyword == head)
    }

    fn strategy(&self, head: &str) -> Option<NameStrategy> {
        self.defs.iter().find(|d| d.keyword == head).map(|d| d.name)
    }

    /// Classify a top-level form's source text, returning `(head_keyword, name)`
    /// if the form is a recognized definition.
    pub fn classify(&self, form_text: &str) -> Option<(String, String)> {
        let inner = form_text.trim_start().strip_prefix('(')?.trim_start();
        let (head, rest) = read_token(inner)?;
        let strat = self.strategy(&head)?;
        let name = match strat {
            NameStrategy::Second => second_name(rest.trim_start())?,
            NameStrategy::LibraryList => library_name(rest.trim_start())?,
        };
        if name.is_empty() {
            None
        } else {
            Some((head, name))
        }
    }
}

/// Read a whitespace/paren-delimited token. Returns `None` for an empty input
/// or one beginning with `(`, `)` or `"`.
fn read_token(input: &str) -> Option<(String, &str)> {
    let input = input.trim_start();
    let first = input.chars().next()?;
    if matches!(first, '(' | ')' | '"') {
        return None;
    }
    let end = input
        .find(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .unwrap_or(input.len());
    if end == 0 {
        None
    } else {
        Some((input[..end].to_string(), &input[end..]))
    }
}

/// Extract the name for [`NameStrategy::Second`] from the text after the head.
fn second_name(rest: &str) -> Option<String> {
    let first = rest.chars().next()?;
    let raw = if first == '(' {
        car_symbol(rest)?
    } else if first == '"' {
        return string_contents(rest);
    } else {
        read_token(rest)?.0
    };
    Some(strip_sigils(&raw))
}

/// The head symbol of a (possibly nested) list: `(a b)` → `a`, `((a) b)` → `a`.
fn car_symbol(list_text: &str) -> Option<String> {
    let inner = list_text.trim_start().strip_prefix('(')?.trim_start();
    if inner.starts_with('(') {
        return car_symbol(inner);
    }
    read_token(inner).map(|(t, _)| t)
}

/// The characters between the first pair of double quotes.
fn string_contents(s: &str) -> Option<String> {
    let s = s.strip_prefix('"')?;
    let end = s.find('"')?;
    Some(s[..end].to_string())
}

/// Join all symbols of a leading list, or read a single symbol.
fn library_name(rest: &str) -> Option<String> {
    if !rest.starts_with('(') {
        return Some(strip_sigils(&read_token(rest)?.0));
    }
    let mut inner = rest.strip_prefix('(')?.trim_start();
    let mut parts = Vec::new();
    while !inner.is_empty() && !inner.starts_with(')') {
        if let Some(stripped) = inner.strip_prefix('(') {
            inner = stripped.trim_start();
            continue;
        }
        match read_token(inner) {
            Some((tok, r)) => {
                parts.push(tok);
                inner = r.trim_start();
            }
            None => break,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Strip a leading quote/quasiquote/keyword sigil (`'foo`, `:foo` → `foo`).
fn strip_sigils(s: &str) -> String {
    s.trim_start_matches(['\'', '`', ':']).to_string()
}

/// True for `(define (name args…) …)` or `(define-values ((a) …) …)`.
pub fn is_curried_define(form_text: &str) -> bool {
    let Some(inner) = form_text.trim_start().strip_prefix('(') else {
        return false;
    };
    let inner = inner.trim_start();
    let Some((head, rest)) = read_token(inner) else {
        return false;
    };
    if !matches!(head.as_str(), "define" | "define-values") {
        return false;
    }
    rest.trim_start().starts_with('(')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> DefinerSet {
        DefinerSet::new(vec![
            Definer::second("defun"),
            Definer::second("defvar"),
            Definer::second("define"),
            Definer::second("defpackage"),
            Definer::second("defalias"),
            Definer::new("define-library", NameStrategy::LibraryList),
        ])
    }

    #[test]
    fn curried_define_detection() {
        assert!(is_curried_define("(define (f x) (+ x 1))"));
        assert!(!is_curried_define("(define x 1)"));
        assert!(is_curried_define("(define-values ((a b)) (+ a b))"));
    }

    #[test]
    fn extend_keywords_adds_custom() {
        let mut s = set();
        s.extend_keywords(&["define-widget".to_string(), "defun".to_string()]);
        assert_eq!(s.classify("(define-widget my-w ...)"), Some(("define-widget".into(), "my-w".into())));
        assert!(s.contains("define-widget"));
    }

    #[test]
    fn simple_second() {
        assert_eq!(set().classify("(defun foo (x) x)"), Some(("defun".into(), "foo".into())));
        assert_eq!(set().classify("(defvar bar 1)"), Some(("defvar".into(), "bar".into())));
    }

    #[test]
    fn curried_define() {
        assert_eq!(set().classify("(define (bar x) (+ x 1))"), Some(("define".into(), "bar".into())));
        assert_eq!(set().classify("(define ((curry x) y) y)"), Some(("define".into(), "curry".into())));
    }

    #[test]
    fn sigils_stripped() {
        assert_eq!(set().classify("(defalias 'a 'b)"), Some(("defalias".into(), "a".into())));
        assert_eq!(set().classify("(defpackage :my-pkg)"), Some(("defpackage".into(), "my-pkg".into())));
        assert_eq!(set().classify("(defpackage \"FOO\" (:use :cl))"), Some(("defpackage".into(), "FOO".into())));
    }

    #[test]
    fn library_list() {
        assert_eq!(set().classify("(define-library (my lib) (export x))"), Some(("define-library".into(), "my lib".into())));
    }

    #[test]
    fn unknown_head_is_none() {
        assert_eq!(set().classify("(require 'cl-lib)"), None);
        assert_eq!(set().classify("(provide 'foo)"), None);
        assert_eq!(set().classify("not-a-list"), None);
    }

    #[test]
    fn contains() {
        assert!(set().contains("defun"));
        assert!(!set().contains("require"));
    }
}
