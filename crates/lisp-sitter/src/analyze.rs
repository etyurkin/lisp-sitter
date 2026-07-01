//! Project-wide semantic analysis: unused definitions, unresolved calls, and
//! arity mismatches across a set of files.
//!
//! This is a best-effort, heuristic analysis — not a full language server.
//! It works purely from tree-sitter / s-expression structure with curated
//! built-in tables, so unresolved-symbol detection in particular can produce
//! false positives (dynamically-built calls, locally-bound functions, builtins
//! outside the curated set). Findings are reported, never applied.

use std::collections::{HashMap, HashSet};

use lisp_sitter_core::sexp_reader::skip_sexp_in;
use lisp_sitter_core::{Dialect, Error, LanguagePlugin, Registry};

use crate::call_scan::{is_call_name, list_children, scan_calls, skip_ws_comments};
use crate::ops;

/// Which checks to run. When all three are `false`, treat it as "run all"
/// (see [`Options::all`]); callers decide that policy.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub unused: bool,
    pub unresolved: bool,
    pub arity: bool,
}

impl Options {
    pub fn all() -> Self {
        Self { unused: true, unresolved: true, arity: true }
    }
}

#[derive(Debug, Clone, Copy)]
struct Arity {
    min: usize,
    /// `None` means variadic (unbounded via `&rest`/`&key`/`. rest`).
    max: Option<usize>,
}

impl Arity {
    fn accepts(&self, n: usize) -> bool {
        n >= self.min && self.max.is_none_or(|m| n <= m)
    }
    fn describe(&self) -> String {
        match self.max {
            None => format!("{}+", self.min),
            Some(m) if m == self.min => format!("{}", self.min),
            Some(m) => format!("{}..{}", self.min, m),
        }
    }
}

struct DefRecord {
    path: String,
    pos: usize,
    head: String,
    arity: Option<Arity>,
    is_macro: bool,
    /// True when the definition is intentionally public and should never be
    /// reported as unused: `;;;###autoload` cookie in elisp, `(export ...)` in
    /// CL, or `(define-library … (export …))` in Scheme.
    is_public: bool,
}

// ── public-API detection ────────────────────────────────────────────

/// True when `;;;###autoload` (or `;;###autoload`) appears on a line by itself
/// in the whitespace/comment block immediately before `form_start`.
fn has_autoload_cookie(content: &str, form_start: usize) -> bool {
    let before = &content[..form_start];
    // Walk backward through whitespace and comments.
    for line in before.lines().rev() {
        let t = line.trim();
        if t.is_empty() || t.starts_with(';') {
            if t.contains("###autoload") { return true; }
        } else {
            break;
        }
    }
    false
}

/// Collect symbol names that are explicitly exported by the file:
/// - Elisp: `;;;###autoload` before a definition (handled per-form via
///   [`has_autoload_cookie`], not this function)
/// - CL: `(export '(foo bar …))` or `(export '(#:foo …))`
/// - Scheme: `(export foo bar …)` inside `(define-library …)`
fn scan_exports(content: &str, plugin_id: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    match plugin_id {
        "commonlisp" => collect_cl_exports(content, &mut out),
        "scheme"     => collect_scheme_exports(content, &mut out),
        _            => {}  // elisp uses autoload cookie per-form
    }
    out
}

fn collect_cl_exports(content: &str, out: &mut HashSet<String>) {
    let b = content.as_bytes();
    let mut i = 0;
    while i < b.len() {
        i = skip_ws_comments(b, i, b.len());
        if i >= b.len() { break; }
        if b[i] != b'(' { let n = skip_sexp_in(b, i, Dialect::Generic).unwrap_or(i + 1); i = n.max(i + 1); continue; }
        let close = skip_sexp_in(b, i, Dialect::Generic).unwrap_or(b.len()).min(b.len());
        let kids = list_children(b, i, close, Dialect::Generic);
        if let Some(&(hs, he)) = kids.first() {
            if content[hs..he].eq_ignore_ascii_case("export") {
                // (export '(foo bar) …) — collect all symbol atoms inside quoted lists
                for (s, e) in kids.iter().skip(1) {
                    collect_symbol_list(content, b, *s, *e, out);
                }
            }
        }
        i = close;
    }
}

fn collect_scheme_exports(content: &str, out: &mut HashSet<String>) {
    // (define-library … (export sym…) …)
    let b = content.as_bytes();
    let mut i = 0;
    while i < b.len() {
        i = skip_ws_comments(b, i, b.len());
        if i >= b.len() { break; }
        if b[i] != b'(' { let n = skip_sexp_in(b, i, Dialect::Generic).unwrap_or(i + 1); i = n.max(i + 1); continue; }
        let close = skip_sexp_in(b, i, Dialect::Generic).unwrap_or(b.len()).min(b.len());
        let kids = list_children(b, i, close, Dialect::Generic);
        if let Some(&(hs, he)) = kids.first() {
            if &content[hs..he] == "define-library" {
                // scan inner forms for (export …)
                for (xs, xe) in kids.iter().skip(1) {
                    if b[*xs] == b'(' {
                        let ik = list_children(b, *xs, *xe, Dialect::Generic);
                        if let Some(&(ihs, ihe)) = ik.first() {
                            if &content[ihs..ihe] == "export" {
                                for (ss, se) in ik.iter().skip(1) {
                                    let sym = content[*ss..*se].trim();
                                    if is_call_name(sym) { out.insert(sym.to_string()); }
                                }
                            }
                        }
                    }
                }
            }
        }
        i = close;
    }
}

/// Collect symbol atoms from a possibly-quoted list `(quote (a b c))` or
/// `'(a b c)` or just a bare atom.  Used to parse `(export '(foo bar))`.
fn collect_symbol_list(content: &str, b: &[u8], s: usize, e: usize, out: &mut HashSet<String>) {
    let s = skip_ws_comments(b, s, e);
    if s >= e { return; }
    // skip leading quote / quasiquote
    let s = if matches!(b[s], b'\'' | b'`') { s + 1 } else { s };
    let s = skip_ws_comments(b, s, e);
    if s >= e { return; }
    if b[s] == b'(' {
        let inner_close = skip_sexp_in(b, s, Dialect::Generic).unwrap_or(e).min(e);
        for (ks, ke) in list_children(b, s, inner_close, Dialect::Generic) {
            let sym = content[ks..ke].trim_start_matches('#').trim_start_matches(':');
            if is_call_name(sym) { out.insert(sym.to_string()); }
        }
    } else {
        // bare atom like `(export #:foo)` or `(export :foo)`
        let sym = content[s..e].trim().trim_start_matches('#').trim_start_matches(':');
        if is_call_name(sym) { out.insert(sym.to_string()); }
    }
}

// ── dependency-prefix collection ────────────────────────────────────

/// Collect the set of package/library prefixes that a file explicitly imports.
/// These are used to suppress unresolved-call warnings for symbols that look
/// like they belong to those packages.
///
/// - Elisp: `(require 'foo-bar)` → prefix `foo-bar`; `(use-package foo-bar)` → same
/// - CL: `:use (:pkg …)` in `defpackage` → prefix `pkg`
/// - Scheme: `(import (srfi 1))` / `(use-modules (ice-9 format))` → last component
fn scan_required_prefixes(content: &str, plugin_id: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let b = content.as_bytes();
    let d = ops::dialect_for_id(plugin_id);
    let mut i = 0;
    while i < b.len() {
        i = skip_ws_comments(b, i, b.len());
        if i >= b.len() { break; }
        if b[i] != b'(' {
            let n = skip_sexp_in(b, i, d).unwrap_or(i + 1);
            i = n.max(i + 1);
            continue;
        }
        let close = skip_sexp_in(b, i, d).unwrap_or(b.len()).min(b.len());
        extract_require_prefixes(content, b, i, close, d, plugin_id, &mut out);
        i = close;
    }
    out
}

fn extract_require_prefixes(content: &str, b: &[u8], open: usize, close: usize, d: Dialect, plugin_id: &str, out: &mut HashSet<String>) {
    let kids = list_children(b, open, close, d);
    let Some(&(hs, he)) = kids.first() else { return };
    let head = &content[hs..he];
    match (plugin_id, head) {
        // (require 'foo-bar) or (require 'foo-bar nil t)
        (_, "require") => {
            if let Some(&(s, e)) = kids.get(1) {
                let sym = unquote_symbol(&content[s..e]);
                if !sym.is_empty() { out.insert(sym.to_string()); }
            }
        }
        // (use-package foo-bar …)
        (_, "use-package") => {
            if let Some(&(s, e)) = kids.get(1) {
                let sym = content[s..e].trim();
                if is_call_name(sym) { out.insert(sym.to_string()); }
            }
        }
        // (defpackage :my-pkg (:use :cl :another))
        ("commonlisp", "defpackage") | ("commonlisp", "define-package") => {
            for (ks, ke) in kids.iter().skip(1) {
                if b[*ks] == b'(' {
                    let ik = list_children(b, *ks, *ke, d);
                    if let Some(&(ihs, ihe)) = ik.first() {
                        let kw = content[ihs..ihe].to_ascii_lowercase();
                        if kw == ":use" || kw == "use" {
                            for (us, ue) in ik.iter().skip(1) {
                                let sym = unquote_symbol(content[*us..*ue].trim());
                                if !sym.is_empty() { out.insert(sym.to_string()); }
                            }
                        }
                    }
                }
            }
        }
        // (import (srfi 1) (scheme base) …) or (use-modules (ice-9 format) …)
        ("scheme", "import") | ("scheme", "use-modules") => {
            for (ks, ke) in kids.iter().skip(1) {
                if b[*ks] == b'(' {
                    // last component of a module spec is often a module name
                    let ik = list_children(b, *ks, *ke, d);
                    if let Some(&(ls, le)) = ik.first() {
                        let sym = content[ls..le].trim();
                        if is_call_name(sym) { out.insert(sym.to_string()); }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Strip leading quote/quasiquote and colon from a symbol token.
fn unquote_symbol(s: &str) -> &str {
    let s = s.trim();
    let s = if let Some(r) = s.strip_prefix('\'') { r } else { s };
    let s = if let Some(r) = s.strip_prefix(':') { r } else { s };
    s.trim()
}

/// True when `symbol` looks like it belongs to a package/prefix in
/// `known_prefixes` — either `prefix-name` (dash-namespaced, elisp style) or
/// `prefix:name` (colon-qualified, CL style) or `prefix/name` (slash, Scheme).
fn matches_required_prefix(symbol: &str, known_prefixes: &HashSet<String>) -> bool {
    for prefix in known_prefixes {
        if symbol.starts_with(&format!("{prefix}-"))
            || symbol.starts_with(&format!("{prefix}:"))
            || symbol.starts_with(&format!("{prefix}/"))
            || symbol.starts_with(&format!("{prefix}::"))
        {
            return true;
        }
        // bare `pkg:sym` where the prefix IS the whole package qualifier
        if let Some(colon) = symbol.find(':') {
            let pkg = symbol[..colon].trim_start_matches(':');
            if pkg.eq_ignore_ascii_case(prefix) { return true; }
        }
    }
    false
}

/// Analyze every file in `paths` and return a human-readable report.
pub fn analyze(reg: &Registry, paths: &[String], opt: Options) -> Result<String, Error> {
    // ── pass 1: read files, collect definitions, exports, required prefixes ──
    let mut files: Vec<(String, String, &dyn LanguagePlugin)> = Vec::new();
    let mut defs: HashMap<String, Vec<DefRecord>> = HashMap::new();
    // Union of all required package prefixes across every file in the project.
    let mut all_required_prefixes: HashSet<String> = HashSet::new();

    for path in paths {
        let Ok(content) = ops::read_source(path, false) else { continue };
        let Ok(plugin) = ops::resolve_plugin(reg, path, None) else { continue };
        let Ok(forms) = plugin.top_level_forms(&content) else { continue };

        let exports = scan_exports(&content, plugin.id());
        let prefixes = scan_required_prefixes(&content, plugin.id());
        all_required_prefixes.extend(prefixes);

        for f in &forms {
            let Some(name) = f.name.clone() else { continue };
            let head = f.label.split(':').next().unwrap_or("").to_string();
            let text = &content[f.start..f.end];
            let is_macro = is_macro_head(&head);
            let arity = function_arity(plugin, &head, plugin.id(), text);
            // A definition is public if it is autoloaded (elisp) or explicitly exported (CL/Scheme).
            let is_public = exports.contains(&name)
                || (plugin.id() == "elisp" && has_autoload_cookie(&content, f.start));
            defs.entry(name).or_default().push(DefRecord {
                path: path.clone(),
                pos: f.start,
                head,
                arity,
                is_macro,
                is_public,
            });
        }
        files.push((path.clone(), content, plugin));
    }

    let mut findings: Vec<(String, usize, String)> = Vec::new();

    // ── unused definitions ──────────────────────────────────────────
    if opt.unused {
        for (name, records) in &defs {
            let candidate = records.iter().any(|r| r.arity.is_some() || r.is_macro);
            if !candidate { continue; }
            // Public API is intentionally unreferenced within the project.
            if records.iter().any(|r| r.is_public) { continue; }
            let total_refs: usize = files.iter().map(|(_, c, p)| p.find_symbol_refs(c, name).len()).sum();
            if total_refs == 0 {
                for r in records {
                    findings.push((
                        r.path.clone(),
                        r.pos,
                        format!("unused: {} `{}` has no references in the analyzed files", r.head, name),
                    ));
                }
            }
        }
    }

    // ── unresolved calls + arity mismatches ─────────────────────────
    if opt.unresolved || opt.arity {
        for (path, content, plugin) in &files {
            let dialect = ops::dialect_for_id(plugin.id());
            let calls = scan_calls(content, dialect);
            for call in calls {
                match defs.get(&call.name) {
                    Some(records) => {
                        if opt.arity && !records.iter().any(|r| r.is_macro) {
                            if let Some(rec) = records.iter().find(|r| r.arity.is_some()) {
                                let a = rec.arity.unwrap();
                                if !a.accepts(call.argc) {
                                    findings.push((
                                        path.clone(),
                                        call.pos,
                                        format!("arity: `{}` called with {} arg(s), expects {}", call.name, call.argc, a.describe()),
                                    ));
                                }
                            }
                        }
                    }
                    None => {
                        if opt.unresolved
                            && !plugin.is_known_global(&call.name)
                            && !matches_required_prefix(&call.name, &all_required_prefixes)
                        {
                            findings.push((
                                path.clone(),
                                call.pos,
                                format!("unresolved: `{}` is not defined in the analyzed files and is not a known builtin", call.name),
                            ));
                        }
                    }
                }
            }
        }
    }

    Ok(render(&files, &mut findings))
}

fn render(files: &[(String, String, &dyn LanguagePlugin)], findings: &mut [(String, usize, String)]) -> String {
    if findings.is_empty() {
        return "No issues found\n".to_string();
    }
    let content_of = |path: &str| files.iter().find(|(p, _, _)| p == path).map(|(_, c, _)| c.as_str());
    findings.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut out = String::new();
    for (path, pos, msg) in findings.iter() {
        let (line, col) = content_of(path).map(|c| lisp_sitter_core::line_column(c, *pos)).unwrap_or((0, 0));
        out.push_str(&format!("{path}:{line}:{col}: {msg}\n"));
    }
    out.push_str(&format!("\n{} issue(s) found\n", findings.len()));
    out
}

fn is_macro_head(head: &str) -> bool {
    matches!(head, "defmacro" | "cl-defmacro" | "define-syntax")
}

/// Compute the arity of a function definition, or `None` when the form is not a
/// non-macro function definition we can reason about.
fn function_arity(plugin: &dyn LanguagePlugin, head: &str, lang: &str, form_text: &str) -> Option<Arity> {
    let is_function = match lang {
        "scheme" => head == "define" && is_curried_define(form_text),
        _ => matches!(head, "defun" | "defsubst" | "cl-defun" | "defmethod" | "defgeneric"),
    };
    if !is_function {
        return None;
    }
    let (params, _) = plugin.form_params_and_body(form_text)?;
    Some(arity_from_params(&params, lang))
}

/// `(define (name args…) …)` — a curried (function) define, as opposed to a
/// value define `(define name value)`.
fn is_curried_define(form_text: &str) -> bool {
    lisp_sitter_core::is_curried_define(form_text)
}

fn arity_from_params(params: &[String], lang: &str) -> Arity {
    if lang == "scheme" {
        // `(a b . rest)` → variadic after the dot; otherwise fixed.
        if let Some(dot) = params.iter().position(|p| p == ".") {
            return Arity { min: dot, max: None };
        }
        return Arity { min: params.len(), max: Some(params.len()) };
    }
    // elisp / commonlisp lambda lists
    let mut min = 0usize;
    let mut max = 0usize;
    let mut optional = false;
    for p in params {
        match p.as_str() {
            "&optional" => optional = true,
            "&rest" | "&body" | "&key" | "&allow-other-keys" | "&aux" | "&environment" | "&whole" => {
                return Arity { min, max: None };
            }
            _ => {
                if optional {
                    max += 1;
                } else {
                    min += 1;
                    max += 1;
                }
            }
        }
    }
    Arity { min, max: Some(max) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_registry;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lisp-sitter-analyze-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &std::path::Path, name: &str, content: &str) -> String {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn arity_required_and_optional() {
        let a = arity_from_params(&["a".into(), "b".into()], "elisp");
        assert!(a.accepts(2));
        assert!(!a.accepts(1));
        assert!(!a.accepts(3));

        let opt = arity_from_params(&["a".into(), "&optional".into(), "b".into()], "elisp");
        assert!(opt.accepts(1));
        assert!(opt.accepts(2));
        assert!(!opt.accepts(3));

        let rest = arity_from_params(&["a".into(), "&rest".into(), "xs".into()], "elisp");
        assert!(rest.accepts(1));
        assert!(rest.accepts(9));
        assert!(!rest.accepts(0));
    }

    #[test]
    fn scheme_variadic_arity() {
        let v = arity_from_params(&["a".into(), ".".into(), "rest".into()], "scheme");
        assert!(v.accepts(1));
        assert!(v.accepts(5));
        assert!(!v.accepts(0));
    }

    #[test]
    fn detects_unused_function() {
        let reg = default_registry();
        let dir = test_dir("unused");
        let p = write(&dir, "a.el", "(defun used () 1)\n(defun lonely () 2)\n(defun caller () (used))\n");
        let report = analyze(&reg, &[p], Options::all()).unwrap();
        assert!(report.contains("unused"), "{report}");
        assert!(report.contains("lonely"), "{report}");
        assert!(!report.contains("`used`"), "{report}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_arity_mismatch() {
        let reg = default_registry();
        let dir = test_dir("arity");
        let p = write(&dir, "a.el", "(defun add (a b) (+ a b))\n(defun go () (add 1))\n");
        let report = analyze(&reg, &[p], Options { unused: false, unresolved: false, arity: true }).unwrap();
        assert!(report.contains("arity"), "{report}");
        assert!(report.contains("`add`"), "{report}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_unresolved_call() {
        let reg = default_registry();
        let dir = test_dir("unresolved");
        let p = write(&dir, "a.el", "(defun go () (frobnicate 1 2))\n");
        let report = analyze(&reg, &[p], Options { unused: false, unresolved: true, arity: false }).unwrap();
        assert!(report.contains("unresolved"), "{report}");
        assert!(report.contains("frobnicate"), "{report}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builtins_not_unresolved() {
        let reg = default_registry();
        let dir = test_dir("builtins");
        let p = write(&dir, "a.el", "(defun go () (message \"%s\" (+ 1 2)))\n");
        let report = analyze(&reg, &[p], Options { unused: false, unresolved: true, arity: false }).unwrap();
        assert!(report.contains("No issues"), "{report}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
