use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use lisp_sitter_core::edit::{ensure_source_editable, get_form_text, insert_after, replace_node};
use lisp_sitter_core::error::{check_ok, syntax_error, syntax_error_node, Error};
use lisp_sitter_core::position::pos_label;
use lisp_sitter_core::{complete_form_in, format_source_in, Dialect, Registry};

fn format_bounds(s: usize, e: usize) -> String {
    format!("{s}:{e}")
}
fn format_check(r: Result<(), Error>) -> String {
    match r {
        Ok(()) => check_ok(),
        Err(Error::Syntax(d)) => syntax_error(d),
        Err(e) => e.to_string(),
    }
}
fn format_check_node(r: Result<(), Error>) -> String {
    match r {
        Ok(()) => check_ok(),
        Err(Error::Syntax(d)) => syntax_error_node(d),
        Err(e) => e.to_string(),
    }
}

pub fn resolve_plugin<'a>(
    reg: &'a Registry,
    path: &str,
    lang: Option<&str>,
) -> Result<&'a dyn lisp_sitter_core::LanguagePlugin, Error> {
    static ENV: OnceLock<String> = OnceLock::new();
    let lang = lang.or_else(|| {
        std::env::var("LISP_SITTER_LANG")
            .ok()
            .map(|v| ENV.get_or_init(|| v).as_str())
    });
    match lang {
        Some(id) => reg
            .plugin_for_id(id)
            .ok_or_else(|| Error::NoPlugin(id.to_string())),
        None => {
            if path == "-" {
                return Err(Error::Message(
                    "provide --lang when reading from stdin".into(),
                ));
            }
            reg.plugin_for_path(path).or_else(|_| {
                read_file(path)
                    .ok()
                    .and_then(|c| detect_language(&c))
                    .and_then(|id| reg.plugin_for_id(id))
                    .ok_or_else(|| Error::NoPlugin(path.to_string()))
            })
        }
    }
}

/// Best-effort content sniffing when the extension doesn't resolve. Checks
/// *dialect-distinctive* forms first (so a Common Lisp file that also uses the
/// shared `defun`/`defvar` isn't misclassified as elisp), then falls back to
/// the shared markers, which lean elisp.
pub fn detect_language(content: &str) -> Option<&'static str> {
    // Distinctive of Common Lisp.
    if content.contains("(defpackage ")
        || content.contains("(in-package ")
        || content.contains("(defclass ")
        || content.contains("(defgeneric ")
        || content.contains("(defmethod ")
        || content.contains("(defparameter ")
        || content.contains("(defconstant ")
    {
        Some("commonlisp")
    // Distinctive of Scheme.
    } else if content.contains("(define-library ")
        || content.contains("(library ")
        || content.contains("(define-record-type ")
    {
        Some("scheme")
    // Shared / elisp-leaning markers.
    } else if content.contains("(defun ")
        || content.contains("(defvar ")
        || content.contains("(provide ")
        || content.contains("(defmacro ")
        || content.contains("(defcustom ")
        || content.contains("(define-minor-mode ")
    {
        Some("elisp")
    } else if content.contains("(define ") || content.contains("(define-syntax ") {
        Some("scheme")
    } else {
        None
    }
}

pub fn tree(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    resolve_plugin(reg, path, None)?.outline(&c)
}
pub fn tree_depth(reg: &Registry, path: &str, depth: usize) -> Result<String, Error> {
    let c = read_file(path)?;
    resolve_plugin(reg, path, None)?.tree_depth(&c, depth)
}

pub fn context(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    let p = resolve_plugin(reg, path, None)?;
    let fs = p.list_forms(&c)?;
    let mut out = String::new();
    out.push_str("-- tree --\n");
    out.push_str(&p.outline(&c)?);
    out.push('\n');
    out.push_str("\n-- forms --\n");
    for f in &fs {
        let label = pos_label(&c, f.start, &f.label);
        let text = &c[f.start..f.end];
        out.push_str(&format!("  {}  {}..{}\n", label, f.start, f.end));
        out.push_str(&format!("  {}\n", text.replace('\n', "\n  ")));
    }
    Ok(out)
}

pub fn bounds(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    let p = resolve_plugin(reg, path, None)?;
    let (s, e) = p.node_bounds(&c, sym)?;
    Ok(format_bounds(s, e))
}

pub fn replace(reg: &Registry, path: &str, sym: &str, body: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    let u = replace_node(resolve_plugin(reg, path, None)?, &c, sym, body)?;
    atomic_write(path, &u)?;
    Ok(format!("Wrote {path}"))
}

pub fn insert(reg: &Registry, path: &str, after: &str, node: &str) -> Result<String, Error> {
    let c = read_file_or_new(path)?;
    let u = insert_after(resolve_plugin(reg, path, None)?, &c, after, node)?;
    atomic_write(path, &u)?;
    Ok(format!("Wrote {path}"))
}

pub fn check_structural_file(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    Ok(format_check(
        resolve_plugin(reg, path, None)?.check_file(&c),
    ))
}

pub fn check_semantic(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    let w = resolve_plugin(reg, path, None)?.semantic_check(&c);
    if w.is_empty() {
        Ok("OK".to_string())
    } else {
        Ok(w.join("\n"))
    }
}

pub fn find_errors(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    let errs = resolve_plugin(reg, path, None)?.find_errors(&c);
    if errs.is_empty() {
        Ok("No errors".to_string())
    } else {
        Ok(errs.join("\n"))
    }
}

pub fn check_structural_node(reg: &Registry, path: &str, node: &str) -> Result<String, Error> {
    Ok(format_check_node(
        resolve_plugin(reg, path, None)?.check_node(node),
    ))
}

pub fn check_node_by_lang(reg: &Registry, lang: &str, body: &str) -> Result<String, Error> {
    Ok(format_check_node(
        reg.plugin_for_id(lang)
            .ok_or_else(|| Error::NoPlugin(lang.to_string()))?
            .check_node(body),
    ))
}

pub fn get_form(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    Ok(get_form_text(resolve_plugin(reg, path, None)?, &c, sym)?.to_string())
}

/// Char-literal flavor for a language id (`elisp` uses `?\(`; others use `#\(`).
pub fn dialect_for_id(id: &str) -> Dialect {
    if id == "elisp" {
        Dialect::Elisp
    } else {
        Dialect::Generic
    }
}

pub fn complete_node(_reg: &Registry, lang: &str, body: &str) -> Result<String, Error> {
    let i = body.trim();
    if i.is_empty() {
        return Err(Error::EmptyForm);
    }
    complete_form_in(i, dialect_for_id(lang))
        .or_else(|| Some(i.to_string()))
        .ok_or_else(|| Error::Message("could not complete".into()))
}

/// Format already-read content. Avoids redundant file I/O for stdin.
pub fn format_content(content: &str, reg: &Registry, path: &str) -> Result<String, Error> {
    let p = resolve_plugin(reg, path, None)?;
    Ok(format_source_in(content, p.dialect()))
}

pub fn format_file(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    format_content(&c, reg, path)
}

pub fn fmt_write(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    let p = resolve_plugin(reg, path, None)?;
    ensure_source_editable(p, &c)?;
    let f = format_source_in(&c, p.dialect());
    p.check_file(&f).map_err(|e| match e {
        Error::Syntax(d) => Error::SyntaxAfterEdit {
            operation: "fmt".into(),
            detail: d,
        },
        o => o,
    })?;
    atomic_write(path, &f)?;
    Ok(format!("Wrote {path}"))
}

/// The confinement root, if `LISP_SITTER_ROOT` is set. When present, every file
/// read/written through ops must resolve to a path inside it — this lets an
/// operator sandbox the MCP server so a tool call can't reach `~/.ssh` or
/// `/etc`. Unset means no confinement (the historical default).
fn confine_root() -> Option<std::path::PathBuf> {
    static ROOT: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::var_os("LISP_SITTER_ROOT").map(|v| {
            let p = std::path::PathBuf::from(v);
            std::fs::canonicalize(&p).unwrap_or(p)
        })
    })
    .clone()
}

/// Resolve `p` to an absolute, symlink-free path for confinement checks,
/// tolerating a not-yet-existing target by canonicalizing its nearest existing
/// ancestor and lexically re-appending the remainder.
fn resolve_for_confinement(p: &Path) -> std::path::PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(c) = std::fs::canonicalize(&cur) {
            let mut r = c;
            for comp in tail.iter().rev() {
                if comp == ".." {
                    r.pop();
                } else if comp != "." {
                    r.push(comp);
                }
            }
            return r;
        }
        match cur.file_name() {
            Some(name) => tail.push(name.to_os_string()),
            None => break,
        }
        if !cur.pop() {
            break;
        }
    }
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

/// Enforce the confinement root (if any). `"-"` (stdin/stdout) is always allowed.
fn check_confined(path: &str) -> Result<(), Error> {
    if path == "-" {
        return Ok(());
    }
    let Some(root) = confine_root() else {
        return Ok(());
    };
    let resolved = resolve_for_confinement(Path::new(path));
    if resolved.starts_with(&root) {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "path `{path}` is outside the confined root `{}`",
            root.display()
        )))
    }
}

/// Read a source file. When `path` is `"-"`, read from stdin.
/// When the file is missing: if `allow_missing` is set and the path has a known
/// Lisp extension, return an empty string (so `insert`/`replace` can create new
/// files); otherwise error.
pub fn read_source(path: &str, allow_missing: bool) -> Result<String, Error> {
    if path == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| Error::Message(format!("stdin: {e}")))?;
        return Ok(buf);
    }
    check_confined(path)?;
    let p = Path::new(path);
    if p.exists() {
        std::fs::read_to_string(p).map_err(|e| Error::Message(format!("read {path}: {e}")))
    } else if allow_missing && is_lisp_ext(path) {
        Ok(String::new())
    } else {
        Err(Error::Message(format!("file not found: {path}")))
    }
}

/// Read for read-only / analysis / edit-existing operations. A missing file is
/// an error — `check`/`tree`/`analyze` on a nonexistent path must not silently
/// report success on empty content.
pub fn read_file(path: &str) -> Result<String, Error> {
    read_source(path, false)
}

/// Read for operations that may create a new file (`insert`). A missing path
/// with a known Lisp extension yields empty content instead of an error.
pub fn read_file_or_new(path: &str) -> Result<String, Error> {
    read_source(path, true)
}

pub fn atomic_write(path: &str, content: &str) -> Result<(), Error> {
    if path == "-" {
        return Err(Error::Message(
            "cannot --write when reading from stdin".into(),
        ));
    }
    check_confined(path)?;
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Message(format!("mkdir {}: {e}", parent.display())))?;
        }
    }
    if p.exists() {
        if let Ok(old) = std::fs::read_to_string(p) {
            if old != content {
                write_backup(p, &old);
            }
        }
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut n = p.file_name().unwrap_or_default().to_os_string();
    n.push(format!(".{ts}.tmp"));
    let tmp = p.with_file_name(&n);
    std::fs::write(&tmp, content)
        .map_err(|e| Error::Message(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, p).map_err(|e| Error::Message(format!("rename {}: {e}", p.display())))?;
    Ok(())
}

/// Save the pre-edit contents of `p` to a per-user backup directory. The
/// directory and file are created with owner-only permissions so source
/// contents (which may include secrets) aren't readable by other local users
/// on a shared machine.
fn write_backup(p: &Path, old: &str) {
    let safe = p.to_string_lossy().replace(['/', ':'], "_");
    let uid = backup_owner_tag();
    let bak_dir = std::env::temp_dir().join(format!("lisp-sitter-backups-{uid}"));
    if std::fs::create_dir_all(&bak_dir).is_err() {
        return;
    }
    restrict_permissions(&bak_dir, 0o700);
    let file = bak_dir.join(format!("{safe}.bak"));
    if std::fs::write(&file, old).is_ok() {
        restrict_permissions(&file, 0o600);
    }
}

/// A per-user tag for the backup directory name, so two users on a shared host
/// don't collide on one directory. The 0700 permissions are the real access
/// control; this only avoids name clashes.
fn backup_owner_tag() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .map(|u| u.replace(|c: char| !c.is_ascii_alphanumeric(), "_"))
        .unwrap_or_else(|_| "user".to_string())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path, _mode: u32) {}

pub fn callers(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = read_file(path)?;
    let p = resolve_plugin(reg, path, None)?;
    let def = p.node_bounds(&c, sym).ok();
    let forms = p.list_forms(&c)?;
    let mut r = Vec::new();
    for sr in p.find_symbol_refs(&c, sym) {
        if sr.kind != lisp_sitter_core::RefKind::CallHead {
            continue;
        }
        let pos = sr.form_start;
        if def.is_some_and(|(ds, de)| pos >= ds && pos < de) {
            continue;
        }
        if let Some(o) = forms.iter().find(|f| pos >= f.start && pos < f.end) {
            r.push(pos_label(&c, pos, &format!("{} calls {}", o.label, sym)));
        }
    }
    if r.is_empty() {
        Ok(format!("No callers of `{sym}` found"))
    } else {
        Ok(r.join("\n"))
    }
}

fn is_lisp_ext(path: &str) -> bool {
    path.ends_with(".el")
        || path.ends_with(".lisp")
        || path.ends_with(".cl")
        || path.ends_with(".scm")
        || path.ends_with(".ss")
        || path.ends_with(".sld")
}

/// Expand a path argument into a concrete list of files.
///
/// - A directory is walked recursively, keeping only Lisp files.
/// - A glob is matched segment-by-segment with shell semantics: `*`/`?` match
///   within a single path component (they do not cross `/`), and `**` matches
///   any number of intervening directories (`src/**/*.el`).
/// - Anything else is returned verbatim as a single-element list.
pub fn expand_paths(reg: &Registry, path: &str) -> Vec<String> {
    if Path::new(path).is_dir() {
        let mut r: Vec<String> = walkdir_paths(path)
            .into_iter()
            .filter(|f| reg.matches_path(f))
            .collect();
        r.sort();
        r
    } else if path.contains('*') || path.contains('?') {
        let mut r = glob_expand(path);
        r.retain(|f| reg.matches_path(f));
        r.sort();
        r.dedup();
        r
    } else {
        vec![path.to_string()]
    }
}

/// Expand a wildcard `pattern` into matching file paths using segment-based
/// matching. `**` matches zero or more directory levels; `*`/`?` stay within
/// one component.
fn glob_expand(pattern: &str) -> Vec<String> {
    let is_abs = pattern.starts_with('/');
    let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let start = if is_abs {
        std::path::PathBuf::from("/")
    } else {
        std::path::PathBuf::from(".")
    };
    let mut out = Vec::new();
    glob_walk(&start, &segments, &mut out);
    out
}

fn glob_walk(dir: &Path, segments: &[&str], out: &mut Vec<String>) {
    let Some((seg, rest)) = segments.split_first() else {
        return;
    };
    if *seg == "**" {
        // `**` matches zero directories (continue here) or one-plus (recurse,
        // keeping `**` so it can match further levels).
        glob_walk(dir, rest, out);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    glob_walk(&p, segments, out);
                }
            }
        }
        return;
    }
    let is_last = rest.is_empty();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !glob_match(seg, &name) {
            continue;
        }
        let p = e.path();
        if is_last {
            if p.is_file() {
                out.push(clean_path(&p));
            }
        } else if p.is_dir() {
            glob_walk(&p, rest, out);
        }
    }
}

/// Stringify a path, stripping the synthetic `./` prefix from relative walks.
fn clean_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    s.strip_prefix("./").map(str::to_string).unwrap_or(s)
}

/// Match a single path component against a glob segment containing `*` and `?`.
fn glob_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star_pi, mut star_ni): (Option<usize>, usize) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = Some(pi);
            star_ni = ni;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ni += 1;
            ni = star_ni;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn walkdir_paths(path: &str) -> Vec<String> {
    let mut r = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(path)];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    r.push(p.to_string_lossy().to_string());
                }
            }
        }
    }
    r
}

pub fn diff_text(old: &str, new: &str, path: &str) -> String {
    let ol: Vec<&str> = old.lines().collect();
    let nl: Vec<&str> = new.lines().collect();
    let max = ol.len().max(nl.len());
    let mut reg: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < max {
        if ol.get(i) != nl.get(i) {
            let s = i;
            while i < max && ol.get(i) != nl.get(i) {
                i += 1;
            }
            reg.push((s, i));
        } else {
            i += 1;
        }
    }
    if reg.is_empty() {
        return String::new();
    }
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    for (rs, re) in &reg {
        let cs = rs.saturating_sub(1);
        let ce = (*re + 1).min(max);
        let old_n = ce.min(ol.len()).saturating_sub(cs);
        let new_n = ce.min(nl.len()).saturating_sub(cs);
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            cs + 1,
            old_n,
            cs + 1,
            new_n
        ));
        for j in cs..ce {
            match (ol.get(j), nl.get(j)) {
                (Some(o), Some(n)) if o == n => out.push_str(&format!(" {o}\n")),
                (Some(o), Some(n)) => {
                    out.push_str(&format!("-{o}\n"));
                    out.push_str(&format!("+{n}\n"));
                }
                (Some(o), None) => out.push_str(&format!("-{o}\n")),
                (None, Some(n)) => out.push_str(&format!("+{n}\n")),
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_registry;

    #[test]
    fn test_detect_language_elisp() {
        assert_eq!(detect_language("(defun foo (x) x)"), Some("elisp"));
        assert_eq!(detect_language("(defvar *x* 1)"), Some("elisp"));
        assert_eq!(detect_language("(provide 'foo)"), Some("elisp"));
        assert_eq!(
            detect_language("(defmacro when (c &rest b) `(if ,c (progn ,@b)))"),
            Some("elisp")
        );
    }

    #[test]
    fn test_detect_language_commonlisp() {
        assert_eq!(detect_language("(defclass foo () ())"), Some("commonlisp"));
        assert_eq!(
            detect_language("(defgeneric process (x))"),
            Some("commonlisp")
        );
        assert_eq!(
            detect_language("(defmethod process ((x integer)) x)"),
            Some("commonlisp")
        );
        assert_eq!(detect_language("(in-package :foo)"), Some("commonlisp"));
    }

    #[test]
    fn test_detect_language_scheme() {
        assert_eq!(detect_language("(define x 1)"), Some("scheme"));
        assert_eq!(detect_language("(define-syntax when ...)"), Some("scheme"));
        assert_eq!(
            detect_language("(define-library (foo) ...)"),
            Some("scheme")
        );
        assert_eq!(detect_language("(library (foo) ...)"), Some("scheme"));
    }

    #[test]
    fn test_detect_language_cl_with_defun() {
        // A Common Lisp file that also uses `defun` must not be seen as elisp.
        assert_eq!(
            detect_language("(defpackage :app (:use :cl))\n(in-package :app)\n(defun f () 1)\n"),
            Some("commonlisp")
        );
    }

    #[test]
    fn test_detect_language_empty_or_unknown() {
        assert_eq!(detect_language(""), None);
        assert_eq!(detect_language("(foo bar baz)"), None);
        assert_eq!(detect_language("just some text"), None);
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*.*", "foo.bar"));
        assert!(glob_match("*.el", "test.el"));
        assert!(!glob_match("*.el", "test.rs"));
        assert!(glob_match("test.*", "test.el"));
        assert!(glob_match("foo*bar", "fooXYZbar"));
        assert!(glob_match("test.el", "test.el"));
        assert!(!glob_match("test.el", "other.el"));
    }

    #[test]
    fn test_expand_paths_single_file() {
        let dir = test_dir("expand_single");
        let path = dir.join("test.el");
        std::fs::write(&path, "(defun foo () 1)\n").unwrap();
        let paths = expand_paths(&default_registry(), path.to_str().unwrap());
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("test.el"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_expand_paths_glob_and_dir() {
        let dir = test_dir("expand_glob");
        std::fs::write(dir.join("a.el"), "(defun a ())\n").unwrap();
        std::fs::write(dir.join("b.el"), "(defun b ())\n").unwrap();
        std::fs::write(dir.join("c.txt"), "text").unwrap();

        let pat = format!("{}/*.el", dir.to_str().unwrap());
        assert_eq!(expand_paths(&default_registry(), &pat).len(), 2);
        // directory expansion keeps only lisp files
        assert_eq!(
            expand_paths(&default_registry(), dir.to_str().unwrap()).len(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_is_lisp_ext() {
        assert!(is_lisp_ext("foo.el"));
        assert!(is_lisp_ext("foo.lisp"));
        assert!(is_lisp_ext("foo.cl"));
        assert!(is_lisp_ext("foo.scm"));
        assert!(is_lisp_ext("foo.ss"));
        assert!(is_lisp_ext("foo.sld"));
        assert!(!is_lisp_ext("foo.txt"));
        assert!(!is_lisp_ext("foo.rs"));
        assert!(!is_lisp_ext("foo"));
    }

    #[test]
    fn test_complete_node() {
        let reg = default_registry();
        assert_eq!(
            &complete_node(&reg, "elisp", "(defun foo (x)").unwrap(),
            "(defun foo (x))"
        );
        // (let ((x 1) requires 3 closing parens: two for (( and one for let
        let result = complete_node(&reg, "elisp", "(let ((x 1").unwrap();
        assert_eq!(result, "(let ((x 1)))");
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("lisp-sitter-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_read_file_new_lisp_file() {
        let dir = test_dir("read_file_new");
        let path = dir.join("test.el");
        // Create-new read yields empty for a missing lisp path...
        assert_eq!(read_file_or_new(path.to_str().unwrap()).unwrap(), "");
        // ...but a strict read errors so check/tree/analyze don't report OK.
        assert!(read_file(path.to_str().unwrap()).is_err());
        let txt = dir.join("foo.txt");
        assert!(read_file_or_new(txt.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_check_missing_file_errors() {
        let reg = default_registry();
        assert!(check_structural_file(&reg, "/nonexistent/missing.el").is_err());
    }

    #[test]
    fn test_glob_single_star_not_recursive() {
        let dir = test_dir("glob_flat");
        std::fs::write(dir.join("a.el"), "(defun a ())\n").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b.el"), "(defun b ())\n").unwrap();
        let pat = format!("{}/*.el", dir.to_str().unwrap());
        let got = expand_paths(&default_registry(), &pat);
        assert_eq!(got.len(), 1, "single * must not descend into sub/: {got:?}");
        assert!(got[0].ends_with("a.el"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_expand_paths_honors_configured_extension() {
        let dir = test_dir("expand_configured_ext");
        std::fs::write(dir.join("a.clef"), "(defun a ())\n").unwrap();
        std::fs::write(dir.join("b.el"), "(defun b ())\n").unwrap();
        // Default registry: only .el is a known extension.
        assert_eq!(
            expand_paths(&default_registry(), dir.to_str().unwrap()).len(),
            1
        );
        // With .clef mapped to commonlisp, both files are included.
        let mut reg = default_registry();
        reg.add_extension(".clef", "commonlisp");
        let got = expand_paths(&reg, dir.to_str().unwrap());
        assert_eq!(
            got.len(),
            2,
            "configured extension must be included: {got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_glob_double_star_recursive() {
        let dir = test_dir("glob_recursive");
        std::fs::write(dir.join("a.el"), "(defun a ())\n").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b.el"), "(defun b ())\n").unwrap();
        let pat = format!("{}/**/*.el", dir.to_str().unwrap());
        let got = expand_paths(&default_registry(), &pat);
        assert_eq!(got.len(), 2, "** must match nested files: {got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_atomic_write_and_read_file() {
        let dir = test_dir("atomic_write");
        let path = dir.join("test.el");

        atomic_write(path.to_str().unwrap(), "(defun foo () 1)").unwrap();
        assert_eq!(
            read_file(path.to_str().unwrap()).unwrap(),
            "(defun foo () 1)"
        );

        atomic_write(path.to_str().unwrap(), "(defun foo () 2)").unwrap();
        assert_eq!(
            read_file(path.to_str().unwrap()).unwrap(),
            "(defun foo () 2)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tree_and_bounds() {
        let reg = default_registry();
        let dir = test_dir("tree_bounds");
        let path = dir.join("test.el");
        atomic_write(
            path.to_str().unwrap(),
            "(defun add (a b)\n  (+ a b))\n\n(defun mul (a b)\n  (* a b))\n",
        )
        .unwrap();

        let t = tree(&reg, path.to_str().unwrap()).unwrap();
        assert!(t.contains("add"));
        assert!(t.contains("mul"));

        let b = bounds(&reg, path.to_str().unwrap(), "add").unwrap();
        assert_eq!(b, "0:27");

        let b2 = bounds(&reg, path.to_str().unwrap(), "mul").unwrap();
        assert_eq!(b2, "29:56");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_form() {
        let reg = default_registry();
        let dir = test_dir("get_form");
        let path = dir.join("test.el");
        atomic_write(
            path.to_str().unwrap(),
            "(defun greet (name)\n  (message \"Hello, %s\" name))\n",
        )
        .unwrap();

        let text = get_form(&reg, path.to_str().unwrap(), "greet").unwrap();
        assert_eq!(text, "(defun greet (name)\n  (message \"Hello, %s\" name))");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_check_structural_file() {
        let reg = default_registry();
        let dir = test_dir("check_file");
        let path = dir.join("test.el");
        atomic_write(path.to_str().unwrap(), "(defun foo () 1)\n").unwrap();

        let result = check_structural_file(&reg, path.to_str().unwrap()).unwrap();
        assert_eq!(result, "OK");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_insert_and_replace() {
        let reg = default_registry();
        let dir = test_dir("insert_replace");
        let path = dir.join("test.el");
        // Start with empty file, insert at __start__
        atomic_write(path.to_str().unwrap(), "").unwrap();
        insert(&reg, path.to_str().unwrap(), "__start__", "(defun a () 1)").unwrap();

        // Insert after 'a'
        insert(&reg, path.to_str().unwrap(), "a", "(defun b () 2)").unwrap();
        let c = read_file(path.to_str().unwrap()).unwrap();
        assert!(c.contains("(defun a () 1)"));
        assert!(c.contains("(defun b () 2)"));

        // Replace 'a'
        replace(&reg, path.to_str().unwrap(), "a", "(defun a () 42)").unwrap();
        let c2 = read_file(path.to_str().unwrap()).unwrap();
        assert!(c2.contains("(defun a () 42)"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_diff_text() {
        let old = "a\nb\nc\n";
        let new = "a\nx\nc\n";
        let d = diff_text(old, new, "f.el");
        assert!(d.contains("-b"));
        assert!(d.contains("+x"));
        assert!(d.contains("f.el"));

        assert_eq!(diff_text("a\nb\n", "a\nb\n", "f.el"), "");

        let d2 = diff_text("a\n", "a\nb\n", "f.el");
        assert!(d2.contains("+b"));
    }

    #[test]
    fn test_callers() {
        let reg = default_registry();
        let dir = test_dir("callers");
        let path = dir.join("test.el");
        atomic_write(
            path.to_str().unwrap(),
            "(defun a ()\n  (b))\n\n(defun b ()\n  1)\n",
        )
        .unwrap();

        let c = callers(&reg, path.to_str().unwrap(), "b").unwrap();
        assert!(c.contains("a calls b"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_plugin_with_path_fallback() {
        let reg = default_registry();
        let dir = test_dir("resolve_fallback");
        let path = dir.join("test.el");
        atomic_write(path.to_str().unwrap(), "(defun foo ()\n  1)\n").unwrap();
        let p = resolve_plugin(&reg, path.to_str().unwrap(), None).unwrap();
        assert_eq!(p.id(), "elisp");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_plugin_by_extension_only() {
        // Non-existent .el file — resolve_plugin should fail because file doesn't exist
        let reg = default_registry();
        let result = resolve_plugin(&reg, "/nonexistent/test.el", None);
        // Without content, it can't detect language by content
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_check_semantic_with_warnings() {
        let reg = default_registry();
        let dir = test_dir("semantic_warn");
        let path = dir.join("test.el");
        // Provide a file that passes structural but has semantic warnings
        atomic_write(path.to_str().unwrap(), "(defun foo ()\n  1)\n").unwrap();
        let r = check_structural_file(&reg, path.to_str().unwrap()).unwrap();
        assert_eq!(r, "OK");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_check_node_by_lang() {
        let reg = default_registry();
        let r = check_node_by_lang(&reg, "elisp", "(defun foo ())").unwrap();
        assert_eq!(r, "OK");
    }

    #[test]
    fn test_check_node_by_lang_invalid() {
        let reg = default_registry();
        let r = check_node_by_lang(&reg, "nonexistent", "(defun foo ())");
        assert!(r.is_err());
    }

    #[test]
    fn test_format_file() {
        let reg = default_registry();
        let dir = test_dir("fmt_file");
        let path = dir.join("test.el");
        atomic_write(path.to_str().unwrap(), "(defun foo ()\n  1)\n").unwrap();
        let r = format_file(&reg, path.to_str().unwrap()).unwrap();
        assert!(r.contains("(defun foo"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fmt_write_roundtrip() {
        let reg = default_registry();
        let dir = test_dir("fmt_write");
        let path = dir.join("test.el");
        atomic_write(path.to_str().unwrap(), "(defun foo ()\n  1)\n").unwrap();
        let r = fmt_write(&reg, path.to_str().unwrap()).unwrap();
        assert!(r.contains("Wrote"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("(defun foo"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fmt_write_malformed_source_refused() {
        let reg = default_registry();
        let dir = test_dir("fmt_write_malformed");
        let path = dir.join("test.el");
        let original = "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n";
        atomic_write(path.to_str().unwrap(), original).unwrap();
        let result = fmt_write(&reg, path.to_str().unwrap());
        assert!(
            matches!(
                result,
                Err(lisp_sitter_core::error::Error::MalformedSource(_))
            ),
            "fmt_write should refuse malformed source: {:?}",
            result
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "file must not be modified"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_plugin_with_unknown_lang() {
        let reg = default_registry();
        let result = resolve_plugin(&reg, "foo.el", Some("nonexistent-lang"));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_plugin_stdin_requires_lang() {
        let reg = default_registry();
        let result = resolve_plugin(&reg, "-", None);
        assert!(matches!(result, Err(Error::Message(msg)) if msg.contains("--lang")));
    }

    #[test]
    fn test_resolve_plugin_fallback_to_content() {
        // Path doesn't match -> fallback to content detection
        // Content doesn't match any known language -> error
        let reg = default_registry();
        let dir = test_dir("resolve_content");
        let path = dir.join("test.xyz"); // unknown extension
        atomic_write(path.to_str().unwrap(), "irrelevant content").unwrap();
        let result = resolve_plugin(&reg, path.to_str().unwrap(), None);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
