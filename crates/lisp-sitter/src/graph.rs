//! Project-wide call graph: callers, callees, explore, impact, and git-diff mapping.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use lisp_sitter_core::position::{line_column, pos_label};
use lisp_sitter_core::{Error, Registry};

use crate::call_scan::scan_calls;
use crate::ops;

const DEFAULT_IMPACT_DEPTH: usize = 5;

#[derive(Debug, Clone)]
struct IndexedFile {
    path: String,
    content: String,
    dialect: lisp_sitter_core::Dialect,
}

#[derive(Debug, Clone)]
struct FormSite {
    file_idx: usize,
    start: usize,
    end: usize,
    label: String,
    name: Option<String>,
}

#[derive(Debug, Clone)]
struct CallEdge {
    caller: usize,
    callee: String,
    call_pos: usize,
}

/// Scan-on-demand call graph over a set of Lisp files.
pub struct ProjectGraph {
    files: Vec<IndexedFile>,
    forms: Vec<FormSite>,
    defs_by_name: HashMap<String, Vec<usize>>,
    edges: Vec<CallEdge>,
}

fn canonical_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

impl ProjectGraph {
    pub fn build(reg: &Registry, paths: &[String]) -> Result<Self, Error> {
        let mut files = Vec::new();
        let mut forms = Vec::new();
        let mut defs_by_name: HashMap<String, Vec<usize>> = HashMap::new();
        let mut edges = Vec::new();

        for path in paths {
            let Ok(content) = ops::read_source(path, false) else {
                continue;
            };
            let canon = canonical_path(path);
            let Ok(plugin) = ops::resolve_plugin(reg, &canon, None) else {
                continue;
            };
            let Ok(file_forms) = plugin.top_level_forms(&content) else {
                continue;
            };

            let file_idx = files.len();
            let dialect = ops::dialect_for_id(plugin.id());
            files.push(IndexedFile {
                path: canon,
                content,
                dialect,
            });

            for f in file_forms {
                let idx = forms.len();
                if let Some(ref name) = f.name {
                    defs_by_name.entry(name.clone()).or_default().push(idx);
                }
                forms.push(FormSite {
                    file_idx,
                    start: f.start,
                    end: f.end,
                    label: f.label,
                    name: f.name,
                });
            }
        }

        for (form_idx, form) in forms.iter().enumerate() {
            let file = &files[form.file_idx];
            let slice = &file.content[form.start..form.end];
            for call in scan_calls(slice, file.dialect) {
                edges.push(CallEdge {
                    caller: form_idx,
                    callee: call.name,
                    call_pos: form.start + call.pos,
                });
            }
        }

        Ok(Self {
            files,
            forms,
            defs_by_name,
            edges,
        })
    }

    fn form(&self, idx: usize) -> &FormSite {
        &self.forms[idx]
    }

    fn file(&self, idx: usize) -> &IndexedFile {
        &self.files[idx]
    }

    fn is_internal_call(&self, edge: &CallEdge, sym: &str) -> bool {
        self.form(edge.caller).name.as_deref() == Some(sym)
    }

    fn format_caller(&self, form_idx: usize, sym: &str) -> String {
        let form = self.form(form_idx);
        let file = self.file(form.file_idx);
        format!(
            "{}:{}: {} calls `{sym}`",
            file.path,
            pos_label(&file.content, form.start, &form.label),
            form.label
        )
    }

    /// Direct callers of `sym` across the indexed files (excludes self-calls in
    /// the definition body).
    pub fn callers(&self, sym: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for edge in &self.edges {
            if edge.callee != sym || self.is_internal_call(edge, sym) {
                continue;
            }
            if seen.insert(edge.caller) {
                out.push(self.format_caller(edge.caller, sym));
            }
        }
        out.sort();
        out
    }

    /// Symbols called directly from the body of `sym`'s definition(s).
    pub fn callees(&self, sym: &str) -> Vec<String> {
        let Some(defs) = self.defs_by_name.get(sym) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for &form_idx in defs {
            for edge in &self.edges {
                if edge.caller != form_idx {
                    continue;
                }
                if seen.insert(edge.callee.clone()) {
                    let file = self.file(self.form(form_idx).file_idx);
                    let (line, col) = line_column(&file.content, edge.call_pos);
                    out.push(format!("{}:{}:{}: `{}`", file.path, line, col, edge.callee));
                }
            }
        }
        out.sort();
        out
    }

    /// Source, definitions, callers, and callees for one symbol.
    pub fn explore(&self, sym: &str) -> String {
        let mut out = String::new();
        out.push_str(&format!("== `{sym}` ==\n\n"));

        if let Some(defs) = self.defs_by_name.get(sym) {
            out.push_str(&format!("definitions ({}):\n", defs.len()));
            for &idx in defs {
                let form = self.form(idx);
                let file = self.file(form.file_idx);
                out.push_str(&format!(
                    "  {}:{}  {}..{}\n",
                    file.path,
                    pos_label(&file.content, form.start, &form.label),
                    form.start,
                    form.end
                ));
            }
            out.push('\n');
            out.push_str("-- source --\n");
            for &idx in defs {
                let form = self.form(idx);
                let file = self.file(form.file_idx);
                let text = &file.content[form.start..form.end];
                out.push_str(&format!(
                    ";; {}\n{}\n",
                    file.path,
                    text.replace('\n', "\n  ")
                ));
            }
        } else {
            out.push_str("definitions: (none in indexed files)\n\n");
        }

        let callers = self.callers(sym);
        out.push_str(&format!("\ncallers ({}):\n", callers.len()));
        if callers.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for line in &callers {
                out.push_str(&format!("  {line}\n"));
            }
        }

        let callees = self.callees(sym);
        out.push_str(&format!("\ncallees ({}):\n", callees.len()));
        if callees.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for line in &callees {
                out.push_str(&format!("  {line}\n"));
            }
        }

        out
    }

    /// Transitive callers up to `max_depth` (blast radius of a change to `sym`).
    pub fn impact(&self, sym: &str, max_depth: usize) -> String {
        let mut out = String::new();
        out.push_str(&format!("impact of `{sym}` (depth {max_depth}):\n\n"));

        if let Some(defs) = self.defs_by_name.get(sym) {
            out.push_str("definitions:\n");
            for &idx in defs {
                let form = self.form(idx);
                let file = self.file(form.file_idx);
                out.push_str(&format!(
                    "  depth 0: {}:{}\n",
                    file.path,
                    pos_label(&file.content, form.start, &form.label)
                ));
            }
            out.push('\n');
        }

        let mut seen_forms: HashSet<usize> = self
            .defs_by_name
            .get(sym)
            .map(|d| d.iter().copied().collect())
            .unwrap_or_default();
        let mut frontier: HashSet<String> = HashSet::from([sym.to_string()]);

        for depth in 1..=max_depth {
            let mut next_frontier = HashSet::new();
            let mut level: Vec<String> = Vec::new();
            for name in &frontier {
                for edge in &self.edges {
                    if edge.callee != *name || self.is_internal_call(edge, name) {
                        continue;
                    }
                    if !seen_forms.insert(edge.caller) {
                        continue;
                    }
                    let form = self.form(edge.caller);
                    let file = self.file(form.file_idx);
                    let line = format!(
                        "  depth {depth}: {}:{}",
                        file.path,
                        pos_label(&file.content, form.start, &form.label)
                    );
                    level.push(line);
                    if let Some(ref n) = form.name {
                        next_frontier.insert(n.clone());
                    }
                }
            }
            if level.is_empty() {
                break;
            }
            level.sort();
            for line in level {
                out.push_str(&line);
                out.push('\n');
            }
            if next_frontier.is_empty() {
                break;
            }
            frontier = next_frontier;
        }

        if out.lines().count() <= 2 {
            out.push_str("  (no callers in indexed files)\n");
        }
        out
    }

    fn forms_touching_line(&self, path: &str, line: u32) -> Vec<usize> {
        let line = line as usize;
        self.forms
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                let file = self.file(f.file_idx);
                file.path == path && line_in_range(&file.content, f.start, f.end, line)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

fn line_in_range(content: &str, start: usize, end: usize, line: usize) -> bool {
    let (s, _) = line_column(content, start);
    let (e, _) = line_column(content, end.min(content.len()));
    line >= s && line <= e
}

fn is_project_path(path: &str) -> bool {
    Path::new(path).is_dir() || path.contains('*') || path.contains('?')
}

fn project_root(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_dir() {
        p.to_path_buf()
    } else if path.contains('*') || path.contains('?') {
        path.rfind('/')
            .map(|i| PathBuf::from(&path[..i]))
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        p.parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn callers(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    if is_project_path(path) {
        let paths = ops::expand_paths(path);
        let graph = ProjectGraph::build(reg, &paths)?;
        let hits = graph.callers(sym);
        if hits.is_empty() {
            Ok(format!("No callers of `{sym}` found in indexed files"))
        } else {
            Ok(hits.join("\n"))
        }
    } else {
        ops::callers(reg, path, sym)
    }
}

pub fn callees(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let paths = ops::expand_paths(path);
    let graph = ProjectGraph::build(reg, &paths)?;
    let hits = graph.callees(sym);
    if hits.is_empty() {
        Ok(format!("No callees from `{sym}` found in indexed files"))
    } else {
        Ok(hits.join("\n"))
    }
}

pub fn explore(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let paths = ops::expand_paths(path);
    let graph = ProjectGraph::build(reg, &paths)?;
    Ok(graph.explore(sym))
}

pub fn impact(reg: &Registry, path: &str, sym: &str, depth: usize) -> Result<String, Error> {
    let paths = ops::expand_paths(path);
    let graph = ProjectGraph::build(reg, &paths)?;
    Ok(graph.impact(sym, depth))
}

/// Map git changes since `base` to touched symbols and optional blast radius.
pub fn diff(
    reg: &Registry,
    path: &str,
    base: &str,
    depth: usize,
    with_impact: bool,
) -> Result<String, Error> {
    let root = std::fs::canonicalize(project_root(path)).unwrap_or_else(|_| project_root(path));
    let paths = ops::expand_paths(path);
    let path_set: HashSet<String> = paths.iter().map(|p| canonical_path(p)).collect();
    let graph = ProjectGraph::build(reg, &paths)?;

    let changed = git_changed_files(&root, base)?;
    let mut out = String::new();
    out.push_str(&format!("diff vs `{base}` (from {})\n\n", root.display()));

    let mut touched: Vec<(String, usize)> = Vec::new();
    let mut changed_lisp = Vec::new();

    for (rel, lines) in &changed {
        let abs = root.join(rel);
        let abs_s = canonical_path(abs.to_str().unwrap());
        if !path_set.contains(&abs_s) {
            continue;
        }
        changed_lisp.push(rel.clone());
        for &line in lines {
            for form_idx in graph.forms_touching_line(&abs_s, line) {
                if let Some(ref name) = graph.form(form_idx).name {
                    touched.push((name.clone(), form_idx));
                }
            }
        }
    }

    out.push_str(&format!("changed lisp files ({}):\n", changed_lisp.len()));
    if changed_lisp.is_empty() {
        out.push_str("  (none in indexed scope)\n");
    } else {
        for f in &changed_lisp {
            out.push_str(&format!("  {f}\n"));
        }
    }

    let mut seen = HashSet::new();
    touched.sort_by(|a, b| a.0.cmp(&b.0));
    out.push_str(&format!("\ntouched symbols ({}):\n", touched.len()));
    if touched.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for (name, form_idx) in touched {
            if !seen.insert((name.clone(), form_idx)) {
                continue;
            }
            let form = graph.form(form_idx);
            let file = graph.file(form.file_idx);
            out.push_str(&format!(
                "  `{}` at {}:{}\n",
                name,
                file.path,
                pos_label(&file.content, form.start, &form.label)
            ));
            if with_impact {
                out.push_str(&graph.impact(&name, depth));
            }
        }
    }

    Ok(out)
}

fn git_changed_files(root: &Path, base: &str) -> Result<Vec<(String, Vec<u32>)>, Error> {
    let names = run_git(root, &["diff", "--name-only", base])?;
    let mut out = Vec::new();
    for rel in names.lines().filter(|l| !l.trim().is_empty()) {
        if !is_lisp_rel(rel) {
            continue;
        }
        let patch = run_git(root, &["diff", "-U0", base, "--", rel])?;
        out.push((rel.to_string(), changed_lines(&patch)));
    }
    Ok(out)
}

fn is_lisp_rel(rel: &str) -> bool {
    rel.ends_with(".el")
        || rel.ends_with(".lisp")
        || rel.ends_with(".cl")
        || rel.ends_with(".scm")
        || rel.ends_with(".ss")
        || rel.ends_with(".sld")
}

fn changed_lines(patch: &str) -> Vec<u32> {
    let mut lines = Vec::new();
    for hunk in patch.lines().filter(|l| l.starts_with("@@")) {
        if let Some(plus) = hunk.split('+').nth(1) {
            let span = plus.split([' ', '@']).next().unwrap_or("");
            let mut parts = span.split(',');
            let start: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let count: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
            for l in start..start + count.max(1) {
                lines.push(l);
            }
        }
    }
    lines
}

fn run_git(root: &Path, args: &[&str]) -> Result<String, Error> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| Error::Message(format!("failed to run git: {e}")))?;
    if !output.status.success() {
        return Err(Error::Message(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn default_impact_depth() -> usize {
    DEFAULT_IMPACT_DEPTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_registry;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("lisp-sitter-graph-{}-{}", std::process::id(), name));
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
    fn project_callers_across_files() {
        let reg = default_registry();
        let dir = test_dir("callers");
        write(&dir, "a.el", "(defun caller () (target))\n");
        write(&dir, "b.el", "(defun other () (target))\n");
        write(&dir, "c.el", "(defun target () 1)\n");
        let out = callers(&reg, dir.to_str().unwrap(), "target").unwrap();
        assert!(out.contains("caller"), "{out}");
        assert!(out.contains("other"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_file_callers_unchanged() {
        let reg = default_registry();
        let dir = test_dir("single");
        let p = write(&dir, "a.el", "(defun a () (b))\n\n(defun b () 1)\n");
        let out = callers(&reg, &p, "b").unwrap();
        assert!(out.contains("a calls b"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn callees_from_definition() {
        let reg = default_registry();
        let dir = test_dir("callees");
        write(
            &dir,
            "a.el",
            "(defun main () (helper (foo)))\n(defun helper (x) x)\n",
        );
        let out = callees(&reg, dir.to_str().unwrap(), "main").unwrap();
        assert!(out.contains("`helper`"), "{out}");
        assert!(out.contains("`foo`"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explore_includes_sections() {
        let reg = default_registry();
        let dir = test_dir("explore");
        write(&dir, "a.el", "(defun f () (g))\n");
        write(&dir, "b.el", "(defun g () 1)\n");
        let out = explore(&reg, dir.to_str().unwrap(), "g").unwrap();
        assert!(out.contains("definitions"), "{out}");
        assert!(out.contains("callers"), "{out}");
        assert!(out.contains("callees"), "{out}");
        assert!(out.contains("f"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn impact_transitive_callers() {
        let reg = default_registry();
        let dir = test_dir("impact");
        write(
            &dir,
            "a.el",
            "(defun top () (mid))\n(defun mid () (leaf))\n(defun leaf () 1)\n",
        );
        let out = impact(&reg, dir.to_str().unwrap(), "leaf", 3).unwrap();
        assert!(out.contains("depth 1"), "{out}");
        assert!(out.contains("mid"), "{out}");
        assert!(out.contains("depth 2"), "{out}");
        assert!(out.contains("top"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_lines_parses_hunk() {
        let patch = "@@ -1,2 +1,3 @@\n";
        let lines = changed_lines(patch);
        assert_eq!(lines, vec![1, 2, 3]);
    }
}
