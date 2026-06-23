//! CLI command handlers. Only contains functions still called from main.rs.
//! Edit operations are inlined in main.rs; MCP handlers call ops:: directly.

use anyhow::Result;
use lisp_sitter_core::error::check_ok;
use lisp_sitter_core::Registry;

use lisp_sitter::ops;

pub fn eval(path: &str) -> Result<()> {
    match lisp_sitter::eval::eval_file(path) {
        Ok((s, e, ok)) => { if !s.is_empty() { print!("{s}"); } if !e.is_empty() { eprint!("{e}"); }
            if ok { println!("OK"); Ok(()) } else { std::process::exit(1); } }
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    }
}

pub fn check(reg: &Registry, path: &str) -> Result<()> {
    for p in expand_paths(path) { let msg = ops::check_structural_file(reg, &p)?;
        if msg == check_ok() { println!("{p}: OK"); } else { eprintln!("{p}: {msg}"); std::process::exit(1); } }
    Ok(())
}

pub fn check_semantic(reg: &Registry, path: &str) -> Result<()> {
    for p in expand_paths(path) { let msg = ops::check_semantic(reg, &p)?;
        if msg.starts_with("OK") { println!("{p}: {msg}"); } else { println!("{p}:\n{msg}"); } }
    Ok(())
}

pub fn wrap(reg: &Registry, path: &str, sym: &str, wrapper: &str, bindings: Option<&str>, cond: Option<&str>, write: bool) -> Result<()> {
    let mut xs: Vec<(&str, &str)> = Vec::new();
    if let Some(b) = bindings { xs.push(("bindings", b)); } if let Some(c) = cond { xs.push(("condition", c)); }
    let u = lisp_sitter::transform::wrap_body(reg, path, sym, wrapper, &xs)?;
    if write { ops::atomic_write(path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); }
    Ok(())
}

pub fn init_git_hook() -> Result<()> {
    let hook_path = std::path::Path::new(".git/hooks/pre-commit");
    if !hook_path.parent().map(|p| p.exists()).unwrap_or(false) { anyhow::bail!("no .git/hooks -- in a git repo?"); }
    let hook = r#"#!/bin/sh
# lisp-sitter pre-commit hook -- check staged Lisp files
set -e
command -v lisp-sitter >/dev/null 2>&1 || exit 0
for f in $(git diff --cached --name-only --diff-filter=ACM | grep -E '\.(el|lisp|cl|scm|ss|sld)$'); do
    if [ -f "$f" ]; then lisp-sitter check "$f" || exit 1; fi
done
"#;
    std::fs::write(hook_path, hook)?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(hook_path, std::fs::Permissions::from_mode(0o755))?; }
    println!("Installed pre-commit hook at {}", hook_path.display()); Ok(())
}

// -- glob / batch helpers ---------------------------------------

fn expand_paths(path: &str) -> Vec<String> {
    use std::path::Path;
    if Path::new(path).is_dir() { let mut r = Vec::new(); if let Ok(e) = walkdir(path) { for f in e { if is_lisp_file(&f) { r.push(f); } } } r.sort(); r }
    else if path.contains('*') || path.contains('?') {
        let mut r = Vec::new();
        let (dir, pat) = match path.rfind('/') { Some(i) => (&path[..i], &path[i+1..]), None => (".", path) };
        if let Ok(e) = walkdir(dir) { for f in e { let n = std::path::Path::new(&f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(); if glob_match(pat, &n) && is_lisp_file(&f) { r.push(f); } } }
        r.sort(); r
    } else { vec![path.to_string()] }
}

fn glob_match(pat: &str, name: &str) -> bool {
    if pat == "*" || pat == "*.*" { return true; } if !pat.contains('*') { return pat == name; }
    let p: Vec<&str> = pat.split('*').collect(); if p.len() == 2 { name.starts_with(p[0]) && name.ends_with(p[1]) } else { p.iter().all(|s| name.contains(s)) }
}

fn is_lisp_file(path: &str) -> bool {
    path.ends_with(".el") || path.ends_with(".lisp") || path.ends_with(".cl") || path.ends_with(".scm") || path.ends_with(".ss") || path.ends_with(".sld")
}

fn walkdir(path: &str) -> std::io::Result<Vec<String>> {
    let mut r = Vec::new(); let mut stack = vec![std::path::PathBuf::from(path)];
    while let Some(dir) = stack.pop() { if let Ok(e) = std::fs::read_dir(&dir) { for entry in e.flatten() { let p = entry.path(); if p.is_dir() { stack.push(p); } else { r.push(p.to_string_lossy().to_string()); } } } }
    Ok(r)
}
