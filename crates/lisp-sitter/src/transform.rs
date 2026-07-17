use lisp_sitter_core::edit::{ensure_source_editable, get_form_text, insert_after, replace_node};
use lisp_sitter_core::form_scan::{
    body_range_char, count_forms, def_params_and_body_char, find_enclosing_sexp, find_sexp_char,
    replace_name_in_form_char, skip_sexp_d, skip_sp, skip_sym, split_elements, substitute_symbol,
    wrap_multi_body,
};
use lisp_sitter_core::plugin::RefKind;
use lisp_sitter_core::sexp_reader::Dialect;
use lisp_sitter_core::{Error, LanguagePlugin, Registry};

// ── helpers ────────────────────────────────────────────────────

fn ops_read(path: &str) -> Result<String, Error> {
    crate::ops::read_source(path, false)
}

/// Sequencing form for multi-expression bodies (`progn` / `begin`).
fn sequence_kw(plugin_id: &str) -> &'static str {
    if plugin_id == "scheme" {
        "begin"
    } else {
        "progn"
    }
}

/// Dialect false literal used as a default `if` else branch.
fn false_lit(plugin_id: &str) -> &'static str {
    if plugin_id == "scheme" {
        "#f"
    } else {
        "nil"
    }
}

// Whitespace and the `(`/`)` delimiters are all ASCII, so these scanners test
// bytes directly. `is_ascii_whitespace()` is false for any UTF-8 continuation
// or lead byte, so a multi-byte symbol (e.g. `xà`) is never split mid-character
// — the returned index always lands on a char boundary.
/// Relabel a post-edit syntax error's operation name. The underlying
/// `replace_node` / `insert_after` already validated the result, so when it is
/// the last mutation we relabel its error instead of re-parsing to re-check.
fn relabel_edit(e: Error, op: &str) -> Error {
    match e {
        Error::SyntaxAfterEdit { detail, .. } => Error::SyntaxAfterEdit {
            operation: op.to_string(),
            detail,
        },
        other => other,
    }
}

/// Try `plugin.find_sexp_in` (tree-sitter, skips strings/comments via AST),
/// falling back to the character scanner only when tree-sitter is unavailable.
fn find_sexp(
    plugin: &dyn LanguagePlugin,
    ft: &str,
    pat: &str,
    d: Dialect,
) -> Option<(usize, usize)> {
    match plugin.find_sexp_in(ft, pat) {
        Some(result) => result, // tree-sitter search completed — trust result
        None => find_sexp_char(ft, pat, d), // parse unavailable — use character scanner
    }
}

/// Find the body byte range within a form, skipping head, name, qualifiers,
/// param list, and any preamble. Uses the plugin's tree-sitter analysis when
/// available; falls back to the character-level scanner otherwise.
fn body_range(plugin: &dyn LanguagePlugin, ft: &str) -> Result<(usize, usize), Error> {
    if let Some((bs, be)) = plugin.form_body_range(ft) {
        if bs <= be && be <= ft.len() {
            return Ok((bs, be));
        }
    }
    body_range_char(ft)
}

/// Controls which `old`-symbol references `replace_head_symbol` renames.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RefsMode {
    /// Only `(old …)` and `#'old` call sites.
    HeadAndSharp,
    /// `(old …)`, `#'old`, and plain `'old` references.
    AllRefs,
    /// Only `(old …)` head positions — no quoted refs at all.
    HeadOnly,
}

/// Replace every occurrence of `old` that matches `refs` mode with `new`,
/// using the plugin's tree-sitter-backed `find_symbol_refs` so that strings,
/// comments, char literals, and binding-var positions are automatically excluded.
/// Replacement is applied back-to-front so byte offsets stay valid.
fn replace_head_symbol(
    plugin: &dyn LanguagePlugin,
    c: &str,
    old: &str,
    new: &str,
    refs: RefsMode,
) -> String {
    let mut positions: Vec<(usize, usize)> = plugin
        .find_symbol_refs(c, old)
        .into_iter()
        .filter(|r| match r.kind {
            RefKind::CallHead => true,
            RefKind::SharpQuote => refs != RefsMode::HeadOnly,
            RefKind::Quote => refs == RefsMode::AllRefs,
        })
        .map(|r| (r.sym_start, r.sym_end))
        .collect();

    // Sort descending so back-to-front replacement preserves earlier offsets.
    positions.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    positions.dedup_by_key(|p| p.0);

    let mut result = c.to_string();
    for (s, e) in positions {
        if s <= e && e <= result.len() {
            result.replace_range(s..e, new);
        }
    }
    result
}

// ── rename ─────────────────────────────────────────────────────

pub fn rename(
    reg: &Registry,
    path: &str,
    old: &str,
    new: &str,
    refs: RefsMode,
) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    rename_content(p, &c, old, new, refs)
}

/// Rename the definition header of `old` plus its references within one file's
/// `content`. Shared by single-file [`rename`] and the per-file pass of
/// [`rename_project`].
fn rename_content(
    p: &dyn LanguagePlugin,
    c: &str,
    old: &str,
    new: &str,
    refs: RefsMode,
) -> Result<String, Error> {
    let u = replace_node_header(p, c, old, new)?;
    let u = replace_head_symbol(p, &u, old, new, refs);
    p.check_file(&u).map_err(|e| match e {
        Error::Syntax(d) => Error::SyntaxAfterEdit {
            operation: "rename".into(),
            detail: d,
        },
        o => o,
    })?;
    Ok(u)
}

/// Does any reference to `old` in `c` fall under the given `refs` mode?
fn touches_symbol(p: &dyn LanguagePlugin, c: &str, old: &str, refs: RefsMode) -> bool {
    p.find_symbol_refs(c, old).iter().any(|r| match r.kind {
        RefKind::CallHead => true,
        RefKind::SharpQuote => refs != RefsMode::HeadOnly,
        RefKind::Quote => refs == RefsMode::AllRefs,
    })
}

/// Project-wide rename across `paths`. Files that define `old` get a full
/// rename (definition header + references); all other files get reference-only
/// updates. Returns `(path, updated_content)` for every file whose content
/// changed — callers decide whether to write or preview. Errors if no file in
/// the set defines `old`.
pub fn rename_project(
    reg: &Registry,
    paths: &[String],
    old: &str,
    new: &str,
    refs: RefsMode,
) -> Result<Vec<(String, String)>, Error> {
    let mut changed = Vec::new();
    let mut found_def = false;
    for path in paths {
        let Ok(c) = ops_read(path) else { continue };
        let Ok(p) = crate::ops::resolve_plugin(reg, path, None) else {
            continue;
        };
        let u = if p.node_bounds(&c, old).is_ok() {
            found_def = true;
            rename_content(p, &c, old, new, refs)?
        } else {
            if !touches_symbol(p, &c, old, refs) {
                continue;
            }
            let u = replace_head_symbol(p, &c, old, new, refs);
            p.check_file(&u).map_err(|e| match e {
                Error::Syntax(d) => Error::SyntaxAfterEdit {
                    operation: "rename".into(),
                    detail: d,
                },
                o => o,
            })?;
            u
        };
        if u != c {
            changed.push((path.clone(), u));
        }
    }
    if !found_def {
        return Err(Error::FormNotFound(old.to_string()));
    }
    Ok(changed)
}

fn replace_node_header(
    p: &dyn lisp_sitter_core::LanguagePlugin,
    c: &str,
    old: &str,
    new: &str,
) -> Result<String, Error> {
    let ft = get_form_text(p, c, old)?;
    let renamed = p
        .form_rename_name(ft, old, new)
        .unwrap_or_else(|| replace_name_in_form_char(ft, old, new));
    replace_node(p, c, old, &renamed)
}

// ── remove ─────────────────────────────────────────────────────

fn remove_form_content(
    p: &dyn LanguagePlugin,
    c: &str,
    sym: &str,
    keep: bool,
) -> Result<String, Error> {
    ensure_source_editable(p, c)?;
    let (s, e) = p.node_bounds(c, sym)?;
    let rs = (0..s)
        .rev()
        .find(|&i| !c.as_bytes()[i].is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(s);
    let mut u = String::with_capacity(c.len());
    u.push_str(&c[..rs]);
    if rs < s {
        u.push('\n');
    }
    u.push_str(&c[e..]);
    if !keep {
        // Stub call sites with the dialect's no-op head symbol.
        u = replace_head_symbol(p, &u, sym, p.noop_stub(), RefsMode::HeadOnly);
    }
    p.check_file(&u).map_err(|e| match e {
        Error::Syntax(d) => Error::SyntaxAfterEdit {
            operation: "remove".into(),
            detail: d,
        },
        o => o,
    })?;
    Ok(u)
}

pub fn remove_form(reg: &Registry, path: &str, sym: &str, keep: bool) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    remove_form_content(p, &c, sym, keep)
}

// ── move ───────────────────────────────────────────────────────

pub fn move_form(reg: &Registry, path: &str, sym: &str, after: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    ensure_source_editable(p, &c)?;
    let ft = get_form_text(p, &c, sym)?.to_string();
    let removed = remove_form_content(p, &c, sym, true)?;
    let ins = insert_after(p, &removed, after, ft.trim()).map_err(|e| relabel_edit(e, "move"))?;
    Ok(ins)
}

// ── substitute ─────────────────────────────────────────────────

pub fn substitute(
    reg: &Registry,
    path: &str,
    sym: &str,
    pat: &str,
    rep: &str,
) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let (s, e) = find_sexp(p, ft, pat, p.dialect())
        .ok_or_else(|| Error::PatternNotFound(pat.to_string()))?;
    let nf = format!("{}{}{}", &ft[..s], rep, &ft[e..]);
    let u = replace_node(p, &c, sym, &nf).map_err(|e| relabel_edit(e, "substitute"))?;
    Ok(u)
}

// ── extract ────────────────────────────────────────────────────

pub fn extract(
    reg: &Registry,
    path: &str,
    sym: &str,
    pat: &str,
    name: &str,
    params: &[&str],
) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let (s, e) = find_sexp(p, ft, pat, p.dialect())
        .ok_or_else(|| Error::PatternNotFound(pat.to_string()))?;
    let ex = &ft[s..e];
    let fv = if params.is_empty() {
        detect_syms(ex, p)
    } else {
        params.to_vec()
    };
    let ps = if fv.is_empty() {
        "()".to_string()
    } else {
        format!("({})", fv.join(" "))
    };
    let nd = p.definition_template(name, &ps, ex);
    let call = if fv.is_empty() {
        format!("({name})")
    } else {
        format!("({} {})", name, fv.join(" "))
    };
    let uf = format!("{}{}{}", &ft[..s], &call, &ft[e..]);
    let as_ = replace_node(p, &c, sym, &uf)?;
    let p2 = crate::ops::resolve_plugin(reg, path, None)?;
    let ins = insert_after(p2, &as_, sym, &nd).map_err(|e| relabel_edit(e, "extract"))?;
    Ok(ins)
}

/// Detect free variable symbols in `sexp` for auto-generating `extract` parameters.
/// Collects all symbol tokens, then filters out those that appear ONLY in call-head
/// position — those are function names (builtins or defined elsewhere), not free variables.
/// Symbols in value/argument/binding positions are kept.
fn detect_syms<'a>(sexp: &'a str, plugin: &dyn LanguagePlugin) -> Vec<&'a str> {
    let mut seen: std::collections::BTreeSet<&str> = Default::default();
    let b = sexp.as_bytes();
    let mut i = 0;
    while i < b.len() {
        i = skip_sp(b, i);
        if i >= b.len() {
            break;
        }
        match b[i] {
            b'(' | b')' => {
                i += 1;
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'\'' | b'`' | b',' => {
                i += 1;
            }
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            _ => {
                let s = i;
                i = skip_sym(b, i);
                let sym = &sexp[s..i];
                if !sym.is_empty()
                    && !sym.starts_with(|c: char| c.is_ascii_digit())
                    && !matches!(sym, "nil" | "t" | "#t" | "#f" | "true" | "false")
                {
                    seen.insert(sym);
                }
            }
        }
    }
    // Filter: keep only symbols that appear in at least one non-call-head position.
    // Symbols that appear exclusively as call heads are function names, not free variables.
    seen.into_iter()
        .filter(|sym| {
            let refs = plugin.find_symbol_refs(sexp, sym);
            refs.is_empty() || refs.iter().any(|r| r.kind != RefKind::CallHead)
        })
        .collect()
}

// ── wrap ────────────────────────────────────────────────────────

pub fn wrap_body(
    reg: &Registry,
    path: &str,
    sym: &str,
    wrapper: &str,
    args: &[(&str, &str)],
) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let b = body_range(p, ft)?;
    let nf = format!(
        "{}{}{}",
        &ft[..b.0],
        make_wrapper(wrapper, args, &ft[b.0..b.1], p.id())?,
        &ft[b.1..]
    );
    replace_node(p, &c, sym, &nf)
}

fn make_wrapper(w: &str, a: &[(&str, &str)], body: &str, plugin_id: &str) -> Result<String, Error> {
    let b = body.trim();
    let seq = sequence_kw(plugin_id);
    match w {
        "progn" | "begin" => Ok(format!("({seq}\n  {})", b.replace('\n', "\n  "))),
        "let" => {
            let bind = a
                .iter()
                .find(|(k, _)| *k == "bindings")
                .map(|(_, v)| *v)
                .unwrap_or("()");
            Ok(format!("(let {}\n  {})", bind, b.replace('\n', "\n  ")))
        }
        "if" => {
            let cond = a
                .iter()
                .find(|(k, _)| *k == "condition")
                .map(|(_, v)| *v)
                .unwrap_or(if plugin_id == "scheme" { "#t" } else { "t" });
            // `if` has fixed arity: the 2nd arg is the whole `then` branch.
            // If the body is more than one form, group it so the extra forms
            // don't silently become `else`/subsequent arguments.
            let then = if count_forms(b) > 1 {
                format!("({seq} {b})")
            } else {
                b.to_string()
            };
            Ok(format!(
                "(if {cond}\n    {}\n  {})",
                then,
                false_lit(plugin_id)
            ))
        }
        o => Err(Error::InvalidArgs(format!("unknown wrapper: {o}"))),
    }
}

// ── instrument ─────────────────────────────────────────────────

pub fn instrument(
    reg: &Registry,
    path: &str,
    sym: &str,
    with: Option<&str>,
    at: Option<&str>,
    wrap: Option<&str>,
) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let nf = if let Some(tf) = with {
        let b = body_range(p, ft)?;
        format!(
            "{}{}{}",
            &ft[..b.0],
            instr_body(&ft[b.0..b.1], tf, p.dialect(), p.id())?,
            &ft[b.1..]
        )
    } else if let (Some(pat), Some(wrp)) = (at, wrap) {
        let (s, e) = find_sexp(p, ft, pat, p.dialect())
            .ok_or_else(|| Error::PatternNotFound(pat.to_string()))?;
        format!("{}{}{}", &ft[..s], &wrp.replace("<form>", pat), &ft[e..])
    } else {
        return Err(Error::InvalidArgs("provide --with or --at --wrap".into()));
    };
    let u = replace_node(p, &c, sym, &nf).map_err(|e| relabel_edit(e, "instrument"))?;
    Ok(u)
}

fn instr_body(body: &str, trace: &str, d: Dialect, plugin_id: &str) -> Result<String, Error> {
    let b = body.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut first = true;
    let seq = sequence_kw(plugin_id);
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let n = skip_sexp_d(b, i, d);
        if n <= i {
            // The scanner couldn't advance over the remaining content (e.g. a
            // char literal the generic scanner mishandled). Fail loudly instead
            // of silently discarding the rest of the function body.
            return Err(Error::Message(
                "could not parse function body for instrumentation".into(),
            ));
        }
        let f = body[i..n].trim();
        if !f.is_empty() {
            if !first {
                out.push('\n');
            }
            out.push_str(&format!("({seq}\n  {}\n  {})", trace, f));
            first = false;
        }
        i = n;
    }
    Ok(out)
}

// ── flatten ────────────────────────────────────────────────────

/// Byte ranges of the elements inside a form `(head e1 e2 …)`, relative to `ft`.
/// Parse a function definition into (param names, single body expression).
/// Uses the plugin's tree-sitter analysis when available; falls back to the
/// character-level scanner. Multi-form bodies are wrapped with `progn`/`begin`.
fn def_params_and_body(
    plugin: &dyn LanguagePlugin,
    ft: &str,
    d: Dialect,
) -> Option<(Vec<String>, String)> {
    if let Some((params, body_text)) = plugin.form_params_and_body(ft) {
        // body_text may be a single expression or multiple; wrap if needed.
        let body = wrap_multi_body(&body_text, d, sequence_kw(plugin.id()));
        return Some((params, body));
    }
    def_params_and_body_char(ft, d)
}

/// Inline every genuine call site of `sym` in `content` by substituting
/// arguments into `body`. Uses the plugin's tree-sitter-backed `find_symbol_refs`
/// so that strings, comments, char literals, and let-binding variable positions
/// are excluded automatically. `def_bounds` excludes the definition form itself
/// (e.g. Scheme curried `(define (sym …) …)` which looks like a call).
/// Replacement is applied back-to-front.
fn inline_calls(
    plugin: &dyn LanguagePlugin,
    content: &str,
    sym: &str,
    params: &[String],
    body: &str,
    d: Dialect,
    def_bounds: Option<(usize, usize)>,
) -> Result<String, Error> {
    let b = content.as_bytes();

    // Collect all genuine call positions, sorted back-to-front.
    let mut call_sites: Vec<usize> = plugin
        .find_symbol_refs(content, sym)
        .into_iter()
        .filter(|r| r.kind == RefKind::CallHead)
        .filter(|r| !def_bounds.is_some_and(|(ds, de)| r.form_start >= ds && r.form_start < de))
        .map(|r| r.form_start)
        .collect();
    call_sites.sort_unstable_by_key(|a| std::cmp::Reverse(*a));

    let mut result = content.to_string();
    for form_start in call_sites {
        let call_end = skip_sexp_d(b, form_start, d);
        if call_end <= form_start || call_end > result.len() {
            continue;
        }
        let call_text = &result[form_start..call_end];
        let args: Vec<String> = split_elements(call_text, d)
            .iter()
            .skip(1)
            .map(|(s, e)| call_text[*s..*e].to_string())
            .collect();
        if args.len() != params.len() {
            return Err(Error::Message(format!(
                "flatten: call to `{sym}` has {} argument(s) but the definition has {} parameter(s)",
                args.len(), params.len()
            )));
        }
        let mut inlined = body.to_string();
        for (pn, av) in params.iter().zip(args.iter()) {
            inlined = substitute_symbol(&inlined, pn, av, d);
        }
        result.replace_range(form_start..call_end, &inlined);
    }
    Ok(result)
}

fn flattenable_params_body(
    p: &dyn LanguagePlugin,
    ft: &str,
    sym: &str,
) -> Result<(Vec<String>, String), Error> {
    let d = p.dialect();
    let (params, body) = def_params_and_body(p, ft, d).ok_or_else(|| {
        Error::Message(format!(
            "flatten: `{sym}` is not a flattenable function definition"
        ))
    })?;
    if params
        .iter()
        .any(|pn| pn.starts_with('&') || pn.starts_with('(') || pn.is_empty())
    {
        return Err(Error::Message(
            "flatten supports only simple positional parameters (no &rest/&optional/&key/destructuring)".into(),
        ));
    }
    if p.find_symbol_refs(&body, sym)
        .iter()
        .any(|r| r.kind == RefKind::CallHead)
    {
        return Err(Error::Message(format!(
            "flatten: `{sym}` is recursive; cannot inline"
        )));
    }
    Ok((params, body))
}

fn remove_definition_form(content: &str, start: usize, end: usize) -> String {
    let rs = (0..start)
        .rev()
        .find(|&i| !content.as_bytes()[i].is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    let mut u = String::with_capacity(content.len());
    u.push_str(&content[..rs]);
    if rs < start {
        u.push('\n');
    }
    u.push_str(&content[end..]);
    u
}

fn flatten_content(
    p: &dyn LanguagePlugin,
    content: &str,
    sym: &str,
    params: &[String],
    body: &str,
    remove_def: bool,
) -> Result<String, Error> {
    ensure_source_editable(p, content)?;
    let d = p.dialect();
    let def_bounds = p.node_bounds(content, sym).ok();
    let inlined = inline_calls(p, content, sym, params, body, d, def_bounds)?;
    let u = if remove_def {
        let (s, e) = p.node_bounds(&inlined, sym)?;
        remove_definition_form(&inlined, s, e)
    } else {
        inlined
    };
    p.check_file(&u).map_err(|e| match e {
        Error::Syntax(detail) => Error::SyntaxAfterEdit {
            operation: "flatten".into(),
            detail,
        },
        o => o,
    })?;
    Ok(u)
}

pub fn flatten(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?.to_string();
    let (params, body) = flattenable_params_body(p, &ft, sym)?;
    flatten_content(p, &c, sym, &params, &body, true)
}

/// Project-wide flatten: inline every call site across `paths`, then remove the
/// definition from the file(s) that define `sym`.
pub fn flatten_project(
    reg: &Registry,
    paths: &[String],
    sym: &str,
) -> Result<Vec<(String, String)>, Error> {
    let mut def: Option<(String, Vec<String>, String)> = None;
    for path in paths {
        let Ok(c) = ops_read(path) else { continue };
        let Ok(p) = crate::ops::resolve_plugin(reg, path, None) else {
            continue;
        };
        if p.node_bounds(&c, sym).is_err() {
            continue;
        }
        let ft = get_form_text(p, &c, sym)?.to_string();
        let (params, body) = flattenable_params_body(p, &ft, sym)?;
        def = Some((path.clone(), params, body));
        break;
    }
    let Some((_def_path, params, body)) = def else {
        return Err(Error::FormNotFound(sym.to_string()));
    };

    let mut changed = Vec::new();
    for path in paths {
        let Ok(c) = ops_read(path) else { continue };
        let Ok(p) = crate::ops::resolve_plugin(reg, path, None) else {
            continue;
        };
        let has_def = p.node_bounds(&c, sym).is_ok();
        let has_calls = p
            .find_symbol_refs(&c, sym)
            .iter()
            .any(|r| r.kind == RefKind::CallHead);
        if !has_def && !has_calls {
            continue;
        }
        let u = flatten_content(p, &c, sym, &params, &body, has_def)?;
        if u != c {
            changed.push((path.clone(), u));
        }
    }
    if changed.is_empty() {
        return Err(Error::FormNotFound(sym.to_string()));
    }
    Ok(changed)
}

// ── splice ─────────────────────────────────────────────────────

/// Remove the outer parentheses of a sub-expression, splicing its children
/// into the parent in its place.  Example: `(progn (a) (b))` → `(a) (b)`.
pub fn splice(reg: &Registry, path: &str, sym: &str, pat: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let d = p.dialect();
    let (s, e) = find_sexp(p, ft, pat, d).ok_or_else(|| Error::PatternNotFound(pat.to_string()))?;
    let b = ft.as_bytes();
    if b.get(s) != Some(&b'(') || b.get(e.saturating_sub(1)) != Some(&b')') {
        return Err(Error::Message(
            "splice: pattern must be a parenthesised list".into(),
        ));
    }
    // Skip the head element (e.g. `progn` in `(progn A B)`) so the body is
    // elevated, not the bare head symbol.
    let after_open = s + 1;
    let ib = &b[after_open..e - 1];
    let mut head_end = 0;
    while head_end < ib.len() && ib[head_end].is_ascii_whitespace() {
        head_end += 1;
    }
    head_end = skip_sexp_d(ib, head_end, d).min(ib.len());
    while head_end < ib.len() && ib[head_end].is_ascii_whitespace() {
        head_end += 1;
    }
    let inner = ft[after_open + head_end..e - 1].trim();
    let nf = format!("{}{}{}", &ft[..s], inner, &ft[e..]);
    let u = replace_node(p, &c, sym, &nf).map_err(|e| relabel_edit(e, "splice"))?;
    Ok(u)
}

// ── raise ──────────────────────────────────────────────────────

/// Replace the direct enclosing list of a sub-expression with just that
/// sub-expression.  Example: `(if cond (bar x) nil)` raise `(bar x)` → `(bar x)`.
pub fn raise(reg: &Registry, path: &str, sym: &str, pat: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let d = p.dialect();
    let (s, e) = find_sexp(p, ft, pat, d).ok_or_else(|| Error::PatternNotFound(pat.to_string()))?;
    let (ps, pe) = find_enclosing_sexp(ft, s, d)
        .ok_or_else(|| Error::Message("raise: pattern has no enclosing form to replace".into()))?;
    let raised = ft[s..e].to_string();
    let nf = format!("{}{}{}", &ft[..ps], raised, &ft[pe..]);
    let u = replace_node(p, &c, sym, &nf).map_err(|e| relabel_edit(e, "raise"))?;
    Ok(u)
}

// ── slurp / barf ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    pub fn parse(s: &str) -> Result<Self, Error> {
        match s {
            "forward" | "f" => Ok(Self::Forward),
            "backward" | "b" => Ok(Self::Backward),
            other => Err(Error::InvalidArgs(format!(
                "invalid direction `{other}`: expected forward or backward"
            ))),
        }
    }
}

fn require_list_pattern(ft: &str, s: usize, e: usize, op: &str) -> Result<(), Error> {
    let b = ft.as_bytes();
    if b.get(s) == Some(&b'(') && b.get(e.saturating_sub(1)) == Some(&b')') {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "{op}: pattern must be a parenthesised list"
        )))
    }
}

fn next_sibling(ft: &str, after: usize, parent_end: usize, d: Dialect) -> Option<(usize, usize)> {
    let b = ft.as_bytes();
    let i = skip_sp(b, after);
    if i >= parent_end.saturating_sub(1) || i >= b.len() {
        return None;
    }
    if b[i] == b')' {
        return None;
    }
    let end = skip_sexp_d(b, i, d);
    if end <= i || end > parent_end {
        return None;
    }
    Some((i, end))
}

fn prev_sibling(
    ft: &str,
    before: usize,
    parent_start: usize,
    d: Dialect,
) -> Option<(usize, usize)> {
    let b = ft.as_bytes();
    let mut last = None;
    let mut i = skip_sp(b, parent_start + 1);
    while i < before {
        if i >= b.len() || b[i] == b')' {
            break;
        }
        let end = skip_sexp_d(b, i, d);
        if end <= i || end > before {
            break;
        }
        last = Some((i, end));
        i = skip_sp(b, end);
    }
    last
}

fn list_elements(ft: &str, s: usize, e: usize, d: Dialect) -> Vec<(usize, usize)> {
    split_elements(&ft[s..e], d)
        .into_iter()
        .map(|(a, b)| (s + a, s + b))
        .collect()
}

/// Paredit slurp: absorb the adjacent sibling into the matched list.
/// Forward: `(list a) b` → `(list a b)`. Backward: `a (list b)` → `(list a b)`.
pub fn slurp(
    reg: &Registry,
    path: &str,
    sym: &str,
    pat: &str,
    dir: Direction,
) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let d = p.dialect();
    let (s, e) = find_sexp(p, ft, pat, d).ok_or_else(|| Error::PatternNotFound(pat.to_string()))?;
    require_list_pattern(ft, s, e, "slurp")?;
    let (ps, pe) = find_enclosing_sexp(ft, s, d).unwrap_or((0, ft.len()));
    let nf = match dir {
        Direction::Forward => {
            let (ns, ne) = next_sibling(ft, e, pe, d).ok_or_else(|| {
                Error::Message("slurp forward: no following sibling to absorb".into())
            })?;
            let inner = ft[s + 1..e - 1].trim_end();
            let sibling = &ft[ns..ne];
            let new_list = if inner.is_empty() {
                format!("({sibling})")
            } else {
                format!("({inner} {sibling})")
            };
            format!("{}{}{}", &ft[..s], new_list, &ft[ne..])
        }
        Direction::Backward => {
            let (ns, ne) = prev_sibling(ft, s, ps, d).ok_or_else(|| {
                Error::Message("slurp backward: no preceding sibling to absorb".into())
            })?;
            let inner = ft[s + 1..e - 1].trim_start();
            let sibling = &ft[ns..ne];
            let new_list = if inner.is_empty() {
                format!("({sibling})")
            } else {
                format!("({sibling} {inner})")
            };
            format!("{}{}{}", &ft[..ns], new_list, &ft[e..])
        }
    };
    let u = replace_node(p, &c, sym, &nf).map_err(|e| relabel_edit(e, "slurp"))?;
    Ok(u)
}

/// Paredit barf: eject the edge element of the matched list as a sibling.
/// Forward: `(list a b)` → `(list a) b`. Backward: `(list a b)` → `a (list b)`.
pub fn barf(
    reg: &Registry,
    path: &str,
    sym: &str,
    pat: &str,
    dir: Direction,
) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let d = p.dialect();
    let (s, e) = find_sexp(p, ft, pat, d).ok_or_else(|| Error::PatternNotFound(pat.to_string()))?;
    require_list_pattern(ft, s, e, "barf")?;
    let elems = list_elements(ft, s, e, d);
    if elems.is_empty() {
        return Err(Error::Message("barf: list has no elements to eject".into()));
    }
    let nf = match dir {
        Direction::Forward => {
            let (es, ee) = *elems.last().unwrap();
            if elems.len() == 1 {
                return Err(Error::Message(
                    "barf forward: refusing to eject the only list element".into(),
                ));
            }
            let kept_end = elems[elems.len() - 2].1;
            let new_list = format!("({})", ft[s + 1..kept_end].trim_end());
            let ejected = &ft[es..ee];
            format!("{}{} {}{}", &ft[..s], new_list, ejected, &ft[e..])
        }
        Direction::Backward => {
            let (es, ee) = elems[0];
            if elems.len() == 1 {
                return Err(Error::Message(
                    "barf backward: refusing to eject the only list element".into(),
                ));
            }
            let kept_start = elems[1].0;
            let new_list = format!("({})", ft[kept_start..e - 1].trim_start());
            let ejected = &ft[es..ee];
            format!("{}{} {}{}", &ft[..s], ejected, new_list, &ft[e..])
        }
    };
    let u = replace_node(p, &c, sym, &nf).map_err(|e| relabel_edit(e, "barf"))?;
    Ok(u)
}

// ── convert-let ────────────────────────────────────────────────

pub fn convert_let(reg: &Registry, path: &str, sym: &str, target: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let (from, to) = match target {
        "let*" => ("let", "let*"),
        "let" => ("let*", "let"),
        other => {
            return Err(Error::InvalidArgs(format!(
                "invalid conversion target `{other}`: expected `let` or `let*`"
            )))
        }
    };
    let ft = get_form_text(p, &c, sym)?;
    // Find the first *syntactic* `(from …)` binding form via the byte scanner,
    // which skips strings and comments — instead of a blind text match that
    // would rewrite a `let` mentioned in a docstring and miss the real form.
    // (`find_symbol_refs` can't be used: tree-sitter classifies `let` as a
    // special form, not a call head, so it returns nothing here.)
    let form_start = lisp_sitter_core::edit::find_callers_in(ft, from, p.dialect())
        .into_iter()
        .min()
        .ok_or_else(|| {
            Error::Message(format!(
                "form `{sym}` contains no `{from}` binding form; cannot convert to {target}"
            ))
        })?;
    let b = ft.as_bytes();
    let mut head = form_start + 1;
    while head < b.len() && b[head].is_ascii_whitespace() {
        head += 1;
    }
    let mut nf = ft.to_string();
    nf.replace_range(head..head + from.len(), to);
    let u = replace_node(p, &c, sym, &nf).map_err(|e| relabel_edit(e, "convert-let"))?;
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_registry;
    use lisp_sitter_core::edit::replace_node;
    use lisp_sitter_core::form_scan::skip_sexp;
    use lisp_sitter_core::Error;

    fn tmp_file(name: &str, content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "lisp-sitter-transform-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.el");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn test_rename() {
        let reg = default_registry();
        let (dir, path) = tmp_file("rename", "(defun foo ()\n  1)\n\n(defun bar ()\n  (foo))\n");
        let result = rename(
            &reg,
            path.to_str().unwrap(),
            "foo",
            "baz",
            RefsMode::HeadAndSharp,
        )
        .unwrap();
        assert!(result.contains("(defun baz ()"));
        assert!(result.contains("(baz)"));
        assert!(result.contains("(defun bar ()"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_form_malformed_source_refused() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "remove_malformed",
            "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n",
        );
        let result = remove_form(&reg, path.to_str().unwrap(), "b", true);
        assert!(
            matches!(result, Err(Error::MalformedSource(_))),
            "remove should refuse malformed source: {:?}",
            result
        );
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("(defun c () 3)"),
            "form c must survive: {content}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_replace_malformed_source_refused() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "replace_malformed",
            "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n",
        );
        let result = replace_node(
            reg.plugin_for_path(path.to_str().unwrap()).unwrap(),
            std::fs::read_to_string(&path).unwrap().as_str(),
            "b",
            "(defun b (x) (+ x 1))",
        );
        assert!(
            matches!(result, Err(Error::MalformedSource(_))),
            "replace should refuse malformed source: {:?}",
            result
        );
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("(defun c () 3)"),
            "form c must survive: {content}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_form() {
        let reg = default_registry();
        let (dir, path) = tmp_file("remove", "(defun foo ()\n  1)\n\n(defun bar ()\n  (foo))\n");
        // remove without keeping calls — foo body replaced with (ignore)
        let result = remove_form(&reg, path.to_str().unwrap(), "foo", false).unwrap();
        assert!(!result.contains("(defun foo"));
        assert!(result.contains("(ignore)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_form_keep_calls() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "remove_keep",
            "(defun foo ()\n  1)\n\n(defun bar ()\n  (foo))\n",
        );
        let result = remove_form(&reg, path.to_str().unwrap(), "foo", true).unwrap();
        assert!(!result.contains("(defun foo"));
        assert!(result.contains("(foo)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_move_form() {
        let reg = default_registry();
        let (dir, path) = tmp_file("move", "(defun a ()\n  1)\n\n(defun b ()\n  2)\n");
        let result = move_form(&reg, path.to_str().unwrap(), "a", "b").unwrap();
        let a_pos = result.find("(defun a").unwrap();
        let b_pos = result.find("(defun b").unwrap();
        assert!(b_pos < a_pos, "a should be moved after b");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_move_form_malformed_source_refused() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "move_malformed",
            "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n",
        );
        let result = move_form(&reg, path.to_str().unwrap(), "a", "c");
        assert!(
            matches!(result, Err(Error::MalformedSource(_))),
            "move should refuse malformed source: {:?}",
            result
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_substitute() {
        let reg = default_registry();
        let (dir, path) = tmp_file("subst", "(defun foo (x)\n  (+ x 1))\n");
        let result = substitute(&reg, path.to_str().unwrap(), "foo", "(+ x 1)", "(* x 2)").unwrap();
        assert!(result.contains("(* x 2)"));
        assert!(!result.contains("(+ x 1)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract() {
        let reg = default_registry();
        let (dir, path) = tmp_file("extract", "(defun foo (x)\n  (+ x 1))\n");
        // extract inserts the new helper after the original definition
        let result = extract(
            &reg,
            path.to_str().unwrap(),
            "foo",
            "(+ x 1)",
            "add1",
            &["x"],
        );
        assert!(result.is_ok(), "extract failed: {:?}", result.err());
        let got = result.unwrap();
        assert!(got.contains("defun add1"), "should define add1: {got}");
        assert!(got.contains("(add1 x)"), "should call add1: {got}");
        // The original (+ x 1) is now only in the extracted definition
        let foo_line: Vec<&str> = got
            .lines()
            .filter(|l| l.trim().starts_with("(defun foo"))
            .collect();
        assert_eq!(foo_line.len(), 1, "foo defined once: {got}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wrap_body_progn() {
        let reg = default_registry();
        let (dir, path) = tmp_file("wrap_progn", "(defun foo ()\n  (+ 1 2))\n");
        let result = wrap_body(&reg, path.to_str().unwrap(), "foo", "progn", &[]).unwrap();
        assert!(result.contains("(progn"));
        assert!(result.contains("(+ 1 2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wrap_body_let() {
        let reg = default_registry();
        let (dir, path) = tmp_file("wrap_let", "(defun foo ()\n  (+ 1 2))\n");
        let result = wrap_body(
            &reg,
            path.to_str().unwrap(),
            "foo",
            "let",
            &[("bindings", "((x 1))")],
        )
        .unwrap();
        assert!(result.contains("(let ((x 1))"));
        assert!(result.contains("(+ 1 2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wrap_body_if() {
        let reg = default_registry();
        let (dir, path) = tmp_file("wrap_if", "(defun foo ()\n  (+ 1 2))\n");
        let result = wrap_body(
            &reg,
            path.to_str().unwrap(),
            "foo",
            "if",
            &[("condition", "(> x 0)")],
        )
        .unwrap();
        assert!(result.contains("(if (> x 0)"));
        assert!(result.contains("(+ 1 2)"));
        assert!(result.contains("nil"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_instrument_with_trace() {
        let reg = default_registry();
        let (dir, path) = tmp_file("instr_with", "(defun foo ()\n  (+ 1 2))\n");
        let result = instrument(
            &reg,
            path.to_str().unwrap(),
            "foo",
            Some("(message \"trace\")"),
            None,
            None,
        )
        .unwrap();
        assert!(result.contains("(progn"));
        assert!(result.contains("(message \"trace\")"));
        assert!(result.contains("(+ 1 2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_instrument_wrap() {
        let reg = default_registry();
        let (dir, path) = tmp_file("instr_wrap", "(defun foo ()\n  (+ 1 2))\n");
        let result = instrument(
            &reg,
            path.to_str().unwrap(),
            "foo",
            None,
            Some("(+ 1 2)"),
            Some("(list <form>)"),
        )
        .unwrap();
        assert!(result.contains("(list (+ 1 2))"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_flatten() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "flatten",
            "(defun add1 (x)\n  (+ x 1))\n\n(defun foo ()\n  (add1 2))\n",
        );
        let result = flatten(&reg, path.to_str().unwrap(), "add1").unwrap();
        // The definition is removed AND the call site is inlined with the
        // argument substituted: (add1 2) -> (+ 2 1).
        assert!(
            !result.contains("(defun add1"),
            "definition should be gone: {result}"
        );
        assert!(
            result.contains("(+ 2 1)"),
            "call should be inlined: {result}"
        );
        assert!(
            !result.contains("(add1 2)"),
            "call should not remain: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_flatten_recursive_errors() {
        let reg = default_registry();
        let (dir, path) = tmp_file("flatten_rec",
            "(defun fact (n)\n  (if (= n 0) 1 (* n (fact (- n 1)))))\n\n(defun use ()\n  (fact 3))\n");
        let result = flatten(&reg, path.to_str().unwrap(), "fact");
        assert!(result.is_err(), "recursive flatten should error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_flatten_arity_mismatch_errors() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "flatten_arity",
            "(defun add (a b)\n  (+ a b))\n\n(defun use ()\n  (add 1))\n",
        );
        let result = flatten(&reg, path.to_str().unwrap(), "add");
        assert!(result.is_err(), "arity mismatch should error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_splice_progn() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "splice_progn",
            "(defun foo (x)\n  (when condition (progn (do-a x) (do-b x))))\n",
        );
        let result = splice(
            &reg,
            path.to_str().unwrap(),
            "foo",
            "(progn (do-a x) (do-b x))",
        )
        .unwrap();
        assert!(
            !result.contains("progn"),
            "progn should be dissolved: {result}"
        );
        assert!(result.contains("(do-a x)"), "do-a must survive: {result}");
        assert!(result.contains("(do-b x)"), "do-b must survive: {result}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_splice_rejects_atom() {
        let reg = default_registry();
        let (dir, path) = tmp_file("splice_atom", "(defun foo (x)\n  (+ x 1))\n");
        let result = splice(&reg, path.to_str().unwrap(), "foo", "x");
        assert!(result.is_err(), "splicing an atom should error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_raise_replaces_enclosing() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "raise_if",
            "(defun foo (x)\n  (if condition (bar x) nil))\n",
        );
        let result = raise(&reg, path.to_str().unwrap(), "foo", "(bar x)").unwrap();
        assert!(result.contains("(bar x)"), "bar must survive: {result}");
        assert!(
            !result.contains("(if condition"),
            "if should be replaced: {result}"
        );
        assert!(
            !result.contains("nil"),
            "else branch should be gone: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_raise_pattern_not_found() {
        let reg = default_registry();
        let (dir, path) = tmp_file("raise_miss", "(defun foo (x)\n  (+ x 1))\n");
        let result = raise(&reg, path.to_str().unwrap(), "foo", "(nonexistent)");
        assert!(result.is_err(), "missing pattern should error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_enclosing_sexp_basic() {
        let text = "(outer (inner) rest)";
        // (inner) starts at 7 (the opening paren)
        let (ps, pe) = find_enclosing_sexp(text, 7, Dialect::Generic).unwrap();
        assert_eq!(ps, 0, "parent should start at 0");
        assert_eq!(pe, text.len(), "parent should end at eof");
    }

    #[test]
    fn test_find_enclosing_sexp_nested() {
        let text = "(a (b (c) d) e)";
        // (c) starts at 6
        let (ps, pe) = find_enclosing_sexp(text, 6, Dialect::Generic).unwrap();
        assert_eq!(&text[ps..pe], "(b (c) d)");
    }

    #[test]
    fn test_find_enclosing_sexp_top_level() {
        let text = "(top-level)";
        // Nothing encloses position 0
        assert!(find_enclosing_sexp(text, 0, Dialect::Generic).is_none());
    }

    #[test]
    fn test_convert_let_to_let_star() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "conv_let",
            "(defun foo ()\n  (let ((x 1) (y 2)) (+ x y)))\n",
        );
        let result = convert_let(&reg, path.to_str().unwrap(), "foo", "let*").unwrap();
        assert!(result.contains("(let*"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_convert_let_star_to_let() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "conv_let*",
            "(defun foo ()\n  (let* ((x 1) (y 2)) (+ x y)))\n",
        );
        let result = convert_let(&reg, path.to_str().unwrap(), "foo", "let").unwrap();
        assert!(result.contains("(let "));
        assert!(!result.contains("(let*"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_convert_let_ignores_docstring_mention() {
        let reg = default_registry();
        let (dir, path) = tmp_file(
            "conv_let_doc",
            "(defun foo ()\n  \"uses (let x) style\"\n  (let ((a 1) (b a)) (+ a b)))\n",
        );
        let result = convert_let(&reg, path.to_str().unwrap(), "foo", "let*").unwrap();
        // The docstring is untouched; the real `let` became `let*`.
        assert!(result.contains("\"uses (let x) style\""));
        assert!(result.contains("(let* ((a 1)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_convert_let_rejects_invalid_target() {
        let reg = default_registry();
        let (dir, path) = tmp_file("conv_let_bad", "(defun foo ()\n  (let* ((x 1)) x))\n");
        assert!(convert_let(&reg, path.to_str().unwrap(), "foo", "letrec").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_non_ascii_symbol_no_panic() {
        let reg = default_registry();
        let (dir, path) = tmp_file("extract_utf8", "(defun my-func (xà) (* xà xà))\n");
        // Must not panic on the multi-byte symbol.
        let r = extract(
            &reg,
            path.to_str().unwrap(),
            "my-func",
            "(* xà xà)",
            "sq",
            &[],
        );
        assert!(r.is_ok() || r.is_err(), "call completed without panicking");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_instrument_elisp_char_literal_preserves_body() {
        let reg = default_registry();
        let (dir, path) = tmp_file("instr_char", "(defun ch () (insert ?\\() (other))\n");
        let r = instrument(
            &reg,
            path.to_str().unwrap(),
            "ch",
            Some("(message \"t\")"),
            None,
            None,
        )
        .unwrap();
        // The body (insert ?\() and (other) must survive, wrapped in progn.
        assert!(r.contains("insert ?\\("), "body form dropped: {r}");
        assert!(r.contains("other"), "trailing form dropped: {r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wrap_if_multiform_body_grouped() {
        let reg = default_registry();
        let (dir, path) = tmp_file("wrap_if_multi", "(defun mf ()\n  (do-a)\n  (do-b))\n");
        let r = wrap_body(
            &reg,
            path.to_str().unwrap(),
            "mf",
            "if",
            &[("condition", "(flag)")],
        )
        .unwrap();
        // Both body forms belong to the then-branch (grouped in progn), not
        // split into then/else.
        assert!(
            r.contains("(progn"),
            "multi-form then should be grouped: {r}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- unit tests for internal helpers --------------------------------

    #[test]
    fn test_find_sexp_basic() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        assert_eq!(
            find_sexp(p, "(+ x 1)", "(+ x 1)", Dialect::Generic),
            Some((0, 7))
        );
        assert_eq!(
            find_sexp(p, "calls (foo) and (bar)", "(foo)", Dialect::Generic),
            Some((6, 11))
        );
    }

    #[test]
    fn test_find_sexp_not_found() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        assert_eq!(
            find_sexp(p, "(defun foo ())", "(bar)", Dialect::Generic),
            None
        );
    }

    #[test]
    fn test_find_sexp_skips_line_comment() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  ; (bar x) in comment\n  (bar x))";
        assert_eq!(
            find_sexp(p, ft, "(bar x)", Dialect::Generic),
            Some((
                ft.rfind("(bar x)").unwrap(),
                ft.rfind("(bar x)").unwrap() + 7
            ))
        );
    }

    #[test]
    fn test_body_range() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  (+ x 1))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_skips_docstring() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  \"docstring\"\n  (+ x 1))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_skips_declare() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  (declare (type integer x))\n  (+ x 1))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_skips_docstring_and_declare() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  \"docstring\"\n  (declare (type integer x))\n  (+ x 1))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_trivial() {
        // () has an empty body between the parens — not an error
        assert!(body_range_char("()").is_ok());
    }

    #[test]
    fn test_body_range_defmethod_with_qualifier() {
        // defmethod with :before qualifier — body must start after the param list, not at :before
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(cl-defmethod my-fn :before ((obj integer) x)\n  (+ obj x))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ obj x)");
    }

    #[test]
    fn test_inline_calls_skips_let_binding_var() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let content = "(defun test () (let ((my-fn 42)) (my-fn 10)))";
        let result = inline_calls(
            p,
            content,
            "my-fn",
            &["x".to_string()],
            "(+ x 1)",
            Dialect::Elisp,
            None,
        )
        .unwrap();
        assert!(
            result.contains("(let ((my-fn 42))"),
            "binding spec must be untouched: {result}"
        );
        assert!(
            result.contains("(+ 10 1)"),
            "call in body must be inlined: {result}"
        );
    }

    #[test]
    fn test_inline_calls_inlines_binding_init() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let content = "(defun test () (let ((x (my-fn 42))) (my-fn x)))";
        let result = inline_calls(
            p,
            content,
            "my-fn",
            &["n".to_string()],
            "(+ n 1)",
            Dialect::Elisp,
            None,
        )
        .unwrap();
        assert!(
            result.contains("(let ((x (+ 42 1)))"),
            "init expression must be inlined: {result}"
        );
        assert!(
            result.contains("(+ x 1)"),
            "call in body must be inlined: {result}"
        );
    }

    #[test]
    fn test_replace_name_in_form_char() {
        let result = replace_name_in_form_char("(defun foo (x) (+ x 1))", "foo", "bar");
        assert_eq!(result, "(defun bar (x) (+ x 1))");
    }

    #[test]
    fn test_replace_name_in_form_no_opener() {
        assert_eq!(
            replace_name_in_form_char("just a string", "x", "y"),
            "just a string"
        );
    }

    #[test]
    fn test_make_wrapper_progn() {
        let result = make_wrapper("progn", &[], "(+ 1 2)", "elisp").unwrap();
        assert_eq!(result, "(progn\n  (+ 1 2))");
    }

    #[test]
    fn test_make_wrapper_begin_scheme() {
        let result = make_wrapper("begin", &[], "(+ 1 2)", "scheme").unwrap();
        assert_eq!(result, "(begin\n  (+ 1 2))");
    }

    #[test]
    fn test_make_wrapper_let() {
        let result = make_wrapper("let", &[("bindings", "((x 1))")], "(+ x 1)", "elisp").unwrap();
        assert_eq!(result, "(let ((x 1))\n  (+ x 1))");
    }

    #[test]
    fn test_make_wrapper_if() {
        let result = make_wrapper("if", &[("condition", "(> x 0)")], "(+ x 1)", "elisp").unwrap();
        assert_eq!(result, "(if (> x 0)\n    (+ x 1)\n  nil)");
    }

    #[test]
    fn test_make_wrapper_if_scheme() {
        let result = make_wrapper("if", &[("condition", "(> x 0)")], "(+ x 1)", "scheme").unwrap();
        assert_eq!(result, "(if (> x 0)\n    (+ x 1)\n  #f)");
    }

    #[test]
    fn test_make_wrapper_unknown() {
        assert!(make_wrapper("unknown", &[], "body", "elisp").is_err());
    }

    #[test]
    fn test_slurp_forward() {
        let reg = default_registry();
        let (dir, path) = tmp_file("slurp_fwd", "(defun foo ()\n  (list a)\n  b)\n");
        let result = slurp(
            &reg,
            path.to_str().unwrap(),
            "foo",
            "(list a)",
            Direction::Forward,
        )
        .unwrap();
        assert!(
            result.contains("(list a b)"),
            "slurp forward failed: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_barf_forward() {
        let reg = default_registry();
        let (dir, path) = tmp_file("barf_fwd", "(defun foo ()\n  (list a b))\n");
        let result = barf(
            &reg,
            path.to_str().unwrap(),
            "foo",
            "(list a b)",
            Direction::Forward,
        )
        .unwrap();
        assert!(
            result.contains("(list a)"),
            "barf should keep list a: {result}"
        );
        assert!(
            result.contains(" b)"),
            "barf should eject b as sibling: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_flatten_project_across_files() {
        let reg = default_registry();
        let dir = std::env::temp_dir().join(format!(
            "lisp-sitter-transform-test-{}-flat-proj",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let def = dir.join("def.el");
        let caller = dir.join("use.el");
        std::fs::write(&def, "(defun add1 (x) (+ x 1))\n").unwrap();
        std::fs::write(&caller, "(defun caller (y) (add1 y))\n").unwrap();
        let paths = vec![
            def.to_str().unwrap().to_string(),
            caller.to_str().unwrap().to_string(),
        ];
        let changed = flatten_project(&reg, &paths, "add1").unwrap();
        assert_eq!(changed.len(), 2);
        let use_out = changed
            .iter()
            .find(|(p, _)| p.ends_with("use.el"))
            .unwrap()
            .1
            .clone();
        assert!(
            use_out.contains("(+ y 1)"),
            "call site should be inlined: {use_out}"
        );
        let def_out = changed
            .iter()
            .find(|(p, _)| p.ends_with("def.el"))
            .unwrap()
            .1
            .clone();
        assert!(
            !def_out.contains("defun add1"),
            "definition should be removed: {def_out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_instrument_scheme_uses_begin() {
        let reg = default_registry();
        let dir = std::env::temp_dir().join(format!(
            "lisp-sitter-transform-test-{}-instr-scm",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.scm");
        std::fs::write(&path, "(define (foo x)\n  (+ x 1))\n").unwrap();
        let result = instrument(
            &reg,
            path.to_str().unwrap(),
            "foo",
            Some("(display \"t\")"),
            None,
            None,
        )
        .unwrap();
        assert!(
            result.contains("(begin"),
            "scheme instrument should use begin: {result}"
        );
        assert!(
            !result.contains("(progn"),
            "scheme instrument must not emit progn: {result}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_syms() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let syms = detect_syms("(+ x 1)", p);
        // `+` appears only as call head → filtered out as function name
        assert!(
            !syms.contains(&"+"),
            "built-in function should be excluded: {syms:?}"
        );
        assert!(!syms.contains(&"1"), "numbers should be excluded: {syms:?}");
        // `x` appears in value position → kept as free variable
        assert!(
            syms.contains(&"x"),
            "free variable x should be detected: {syms:?}"
        );
    }

    #[test]
    fn test_replace_head_symbol() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let c = "(foo 1)\n(bar (foo 2))\n(ignore)";
        let result = replace_head_symbol(p, c, "foo", "baz", RefsMode::HeadOnly);
        assert_eq!(result, "(baz 1)\n(bar (baz 2))\n(ignore)");
    }

    #[test]
    fn test_replace_head_symbol_skips_strings_and_comments() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let c = "(foo)\n(bar (message \"call (foo) here\")) ; foo in comment\n";
        let result = replace_head_symbol(p, c, "foo", "baz", RefsMode::HeadOnly);
        assert!(result.contains("(baz)"));
        assert!(
            result.contains("\"call (foo) here\""),
            "string must be untouched: {result}"
        );
        assert!(
            result.contains("; foo in comment"),
            "comment must be untouched: {result}"
        );
    }

    #[test]
    fn test_replace_head_symbol_sharp_refs() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let c = "(foo)\n(add-hook 'h #'foo)\n(setq x 'foo)";
        let got = replace_head_symbol(p, c, "foo", "baz", RefsMode::HeadAndSharp);
        assert!(got.contains("(baz)"), "{got}");
        assert!(got.contains("#'baz"), "#' should be renamed: {got}");
        assert!(
            got.contains("'foo"),
            "plain 'foo should not be renamed without --refs: {got}"
        );
    }

    #[test]
    fn test_replace_head_symbol_all_refs() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let c = "(foo)\n(add-hook 'h #'foo)\n(setq sym 'foo)";
        let got = replace_head_symbol(p, c, "foo", "baz", RefsMode::AllRefs);
        assert!(got.contains("(baz)"), "{got}");
        assert!(got.contains("#'baz"), "{got}");
        assert!(
            got.contains("'baz"),
            "plain 'foo should be renamed with --refs: {got}"
        );
    }

    #[test]
    fn test_skip_sexp_parens() {
        let b = b"(defun foo (x) x)";
        let end = skip_sexp(b, 0);
        assert_eq!(end, b.len());
    }

    #[test]
    fn test_skip_sexp_empty() {
        assert_eq!(skip_sexp(b"", 0), 0);
    }

    #[test]
    fn test_skip_sexp_symbol() {
        let b = b"foo bar";
        let end = skip_sexp(b, 0);
        assert_eq!(&b[0..end], b"foo");
    }

    #[test]
    fn test_skip_sexp_string() {
        let b = b"(\"hello\")";
        let end = skip_sexp(b, 0);
        assert_eq!(end, b.len());
    }

    #[test]
    fn test_skip_sp() {
        assert_eq!(skip_sp(b"   abc", 0), 3);
        assert_eq!(skip_sp(b"abc", 0), 0);
    }

    #[test]
    fn test_skip_sym() {
        let b = b"foo bar";
        assert_eq!(&b[0..skip_sym(b, 0)], b"foo");
        assert_eq!(skip_sym(b"  foo", 2), 5);
    }

    #[test]
    fn test_ops_read_nonexistent_txt() {
        let result = ops_read("/nonexistent-file-for-test.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_instrument_no_args_error() {
        let reg = crate::default_registry();
        let (dir, path) = tmp_file("instr_err", "(defun foo ()\n  1)\n");
        // No --with and no --at/--wrap
        let result = instrument(&reg, path.to_str().unwrap(), "foo", None, None, None);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_syms_string_and_comment() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let syms = detect_syms(
            r#"(some-fn "string with (parens)" ; comment
  x)"#,
            p,
        );
        assert!(
            !syms.contains(&"+"),
            "non-existent + should not be in syms: {syms:?}"
        );
        // some-fn appears only as a call head → filtered as function name
        assert!(
            !syms.contains(&"some-fn"),
            "call-head function should be excluded: {syms:?}"
        );
        // x appears as a value → included
        assert!(
            syms.contains(&"x"),
            "free variable x should be detected: {syms:?}"
        );
    }

    #[test]
    fn test_replace_name_in_form_inner_paren() {
        // Form like (defmethod foo ((x integer) ...)) — inner parens before name
        let result = replace_name_in_form_char("(defmethod foo ((x integer) body)", "foo", "bar");
        assert_eq!(result, "(defmethod bar ((x integer) body)");
    }

    #[test]
    fn test_replace_name_in_form_no_name_match() {
        let result = replace_name_in_form_char("(foo bar baz)", "qux", "quux");
        assert_eq!(result, "(foo bar baz)");
    }

    #[test]
    fn test_replace_name_in_form_inner_paren_with_match() {
        // ah starts with '(' and inner name matches (defmethod-like)
        let result = replace_name_in_form_char("(defmethod foo ((x integer)) body)", "foo", "bar");
        assert_eq!(result, "(defmethod bar ((x integer)) body)");
    }

    #[test]
    fn test_detect_syms_with_quote() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let syms = detect_syms("'(1 2 3)", p);
        assert!(syms.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn test_detect_syms_excludes_builtins() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let syms = detect_syms("(format t \"%s\" x)", p);
        // `format` and `t` appear only as call head / keyword → excluded
        assert!(
            !syms.contains(&"format"),
            "format (builtin) should be excluded: {syms:?}"
        );
        // `x` appears in value position → included
        assert!(
            syms.contains(&"x"),
            "free variable x should be included: {syms:?}"
        );
    }
}
