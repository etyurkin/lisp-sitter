use std::path::Path;
use std::sync::OnceLock;

use lisp_sitter_core::edit::{get_form_text, insert_after, replace_node};
use lisp_sitter_core::error::{check_ok, syntax_error, syntax_error_node, Error};
use lisp_sitter_core::position::pos_label;
use lisp_sitter_core::{complete_form, format_source, Registry};

fn format_bounds(s: usize, e: usize) -> String { format!("{s}:{e}") }
fn format_check(r: Result<(), Error>) -> String { match r { Ok(()) => check_ok(), Err(Error::Syntax(d)) => syntax_error(d), Err(e) => e.to_string() } }
fn format_check_node(r: Result<(), Error>) -> String { match r { Ok(()) => check_ok(), Err(Error::Syntax(d)) => syntax_error_node(d), Err(e) => e.to_string() } }

pub fn resolve_plugin<'a>(reg: &'a Registry, path: &str, lang: Option<&str>) -> Result<&'a dyn lisp_sitter_core::LanguagePlugin, Error> {
    static ENV: OnceLock<String> = OnceLock::new();
    let lang = lang.or_else(|| std::env::var("LISP_SITTER_LANG").ok().and_then(|v| Some(ENV.get_or_init(|| v).as_str())));
    match lang {
        Some(id) => reg.plugin_for_id(id).ok_or_else(|| Error::NoPlugin(id.to_string())),
        None => reg.plugin_for_path(path).or_else(|_| {
            read_file(path).ok().and_then(|c| detect_language(&c)).and_then(|id| reg.plugin_for_id(id))
                .ok_or_else(|| Error::NoPlugin(path.to_string()))
        }),
    }
}

pub fn detect_language(content: &str) -> Option<&'static str> {
    if content.contains("(defun ") || content.contains("(defvar ") || content.contains("(provide ") || content.contains("(defmacro ") { Some("elisp") }
    else if content.contains("(defclass ") || content.contains("(defgeneric ") || content.contains("(defmethod ") || content.contains("(in-package ") { Some("commonlisp") }
    else if content.contains("(define ") || content.contains("(define-syntax ") || content.contains("(define-library ") || content.contains("(library ") { Some("scheme") }
    else { None }
}

pub fn tree(reg: &Registry, path: &str) -> Result<String, Error> { let c = read_file(path)?; resolve_plugin(reg, path, None)?.outline(&c) }
pub fn tree_depth(reg: &Registry, path: &str, depth: usize) -> Result<String, Error> { let c = read_file(path)?; resolve_plugin(reg, path, None)?.tree_depth(&c, depth) }

pub fn context(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?; let p = resolve_plugin(reg, path, None)?; let fs = p.list_forms(&c)?;
    let mut out = String::new(); out.push_str("-- tree --\n"); out.push_str(&p.outline(&c)?); out.push('\n'); out.push_str("\n-- forms --\n");
    for f in &fs { let label = pos_label(&c, f.start, &f.label); let text = &c[f.start..f.end];
        out.push_str(&format!("  {}  {}..{}\n", label, f.start, f.end)); out.push_str(&format!("  {}\n", text.replace('\n', "\n  "))); }
    Ok(out)
}

pub fn bounds(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = read_file(path)?; let p = resolve_plugin(reg, path, None)?; let (s, e) = p.node_bounds(&c, sym)?; Ok(format_bounds(s, e))
}

pub fn replace(reg: &Registry, path: &str, sym: &str, body: &str) -> Result<String, Error> {
    let c = read_file(path)?; let u = replace_node(resolve_plugin(reg, path, None)?, &c, sym, body)?; atomic_write(path, &u)?; Ok(format!("Wrote {path}"))
}

pub fn insert(reg: &Registry, path: &str, after: &str, node: &str) -> Result<String, Error> {
    let c = read_file(path)?; let u = insert_after(resolve_plugin(reg, path, None)?, &c, after, node)?; atomic_write(path, &u)?; Ok(format!("Wrote {path}"))
}

pub fn check_structural_file(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?; Ok(format_check(resolve_plugin(reg, path, None)?.check_file(&c)))
}

pub fn check_semantic(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?; let w = resolve_plugin(reg, path, None)?.semantic_check(&c);
    if w.is_empty() { Ok("OK".to_string()) } else { Ok(w.join("\n")) }
}

pub fn check_structural_node(reg: &Registry, path: &str, node: &str) -> Result<String, Error> {
    Ok(format_check_node(resolve_plugin(reg, path, None)?.check_node(node)))
}

pub fn check_node_by_lang(reg: &Registry, lang: &str, body: &str) -> Result<String, Error> {
    Ok(format_check_node(reg.plugin_for_id(lang).ok_or_else(|| Error::NoPlugin(lang.to_string()))?.check_node(body)))
}

pub fn get_form(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = read_file(path)?; Ok(get_form_text(resolve_plugin(reg, path, None)?, &c, sym)?.to_string())
}

pub fn complete_node(_reg: &Registry, _lang: &str, body: &str) -> Result<String, Error> {
    let i = body.trim(); if i.is_empty() { return Err(Error::EmptyForm); }
    complete_form(i).or_else(|| Some(i.to_string())).ok_or_else(|| Error::Message("could not complete".into()))
}

pub fn format_file(reg: &Registry, path: &str) -> Result<String, Error> { let c = read_file(path)?; let _ = resolve_plugin(reg, path, None)?; Ok(format_source(&c)) }

pub fn fmt_write(reg: &Registry, path: &str) -> Result<String, Error> {
    let c = read_file(path)?; let p = resolve_plugin(reg, path, None)?; let f = format_source(&c);
    p.check_file(&f).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "fmt".into(), detail: d }, o => o })?;
    atomic_write(path, &f)?; Ok(format!("Wrote {path}"))
}

pub fn read_file(path: &str) -> Result<String, Error> {
    let p = Path::new(path);
    if p.exists() { std::fs::read_to_string(p).map_err(|e| Error::Message(format!("read {path}: {e}"))) }
    else if is_lisp_ext(path) { Ok(String::new()) }
    else { Err(Error::Message(format!("file not found: {path}"))) }
}

pub fn atomic_write(path: &str, content: &str) -> Result<(), Error> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() { if !parent.as_os_str().is_empty() { std::fs::create_dir_all(parent).map_err(|e| Error::Message(format!("mkdir {}: {e}", parent.display())))?; } }
    if p.exists() { if let Ok(old) = std::fs::read_to_string(p) { if old != content {
        let safe = p.to_string_lossy().replace('/', "_").replace(':', "_");
        let bak_dir = std::env::temp_dir().join("lisp-sitter-backups"); let _ = std::fs::create_dir_all(&bak_dir);
        let _ = std::fs::write(&bak_dir.join(format!("{}.bak", safe)), &old);
    }}}
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let mut n = p.file_name().unwrap_or_default().to_os_string(); n.push(format!(".{ts}.tmp"));
    let tmp = p.with_file_name(&n);
    std::fs::write(&tmp, content).map_err(|e| Error::Message(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, p).map_err(|e| Error::Message(format!("rename {}: {e}", p.display())))?;
    Ok(())
}

pub fn callers(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = read_file(path)?; let p = resolve_plugin(reg, path, None)?; let def = p.node_bounds(&c, sym).ok();
    let pat = format!("({} ", sym); let pat2 = format!("({})", sym); let mut r = Vec::new(); let mut s = 0;
    loop { let pos = if let Some(pos) = c[s..].find(&pat) { s + pos } else if let Some(pos) = c[s..].find(&pat2) { s + pos } else { break };
        if def.map_or(false, |(ds, de)| pos >= ds && pos < de) { s = pos + 1; continue; }
        if let Some(o) = p.list_forms(&c)?.iter().find(|f| pos >= f.start && pos < f.end) { r.push(pos_label(&c, pos, &format!("{} calls {}", o.label, sym))); }
        s = pos + 1; }
    if r.is_empty() { Ok(format!("No callers of `{sym}` found")) } else { Ok(r.join("\n")) }
}

fn is_lisp_ext(path: &str) -> bool {
    path.ends_with(".el") || path.ends_with(".lisp") || path.ends_with(".cl")
        || path.ends_with(".scm") || path.ends_with(".ss") || path.ends_with(".sld")
}

pub fn diff_text(old: &str, new: &str, path: &str) -> String {
    let ol: Vec<&str> = old.lines().collect(); let nl: Vec<&str> = new.lines().collect();
    let max = ol.len().max(nl.len()); let mut reg: Vec<(usize, usize)> = Vec::new(); let mut i = 0;
    while i < max { if ol.get(i) != nl.get(i) { let s = i; while i < max && ol.get(i) != nl.get(i) { i += 1; } reg.push((s, i)); } else { i += 1; } }
    if reg.is_empty() { return String::new(); }
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    for (rs, re) in &reg { let cs = rs.saturating_sub(1); let ce = (*re + 1).min(max);
        out.push_str(&format!("@@ -{},{} +{},{} @@\n", cs + 1, ce - cs, cs + 1, ce - cs));
        for j in cs..ce { match (ol.get(j), nl.get(j)) { (Some(o), Some(n)) if o == n => out.push_str(&format!(" {o}\n")), (Some(o), Some(n)) => { out.push_str(&format!("-{o}\n")); out.push_str(&format!("+{n}\n")); } (Some(o), None) => out.push_str(&format!("-{o}\n")), (None, Some(n)) => out.push_str(&format!("+{n}\n")), _ => {} } }
    }
    out
}
