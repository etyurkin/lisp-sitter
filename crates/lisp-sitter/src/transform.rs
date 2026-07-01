use lisp_sitter_core::edit::{ensure_source_editable, get_form_text, insert_after, replace_node};
use lisp_sitter_core::plugin::RefKind;
use lisp_sitter_core::sexp_reader::Dialect;
use lisp_sitter_core::{Error, LanguagePlugin, Registry};

// ── helpers ────────────────────────────────────────────────────

fn ops_read(path: &str) -> Result<String, Error> {
    crate::ops::read_source(path, false)
}

/// Char-literal flavor for a plugin (elisp uses `?\(`; CL/Scheme use `#\(`).
fn dialect_of(p: &dyn LanguagePlugin) -> Dialect {
    if p.id() == "elisp" { Dialect::Elisp } else { Dialect::Generic }
}

fn skip_sp(bytes: &[u8], mut i: usize) -> usize { while i < bytes.len() && (bytes[i] as char).is_whitespace() { i += 1; } i }
fn skip_sym(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && !(bytes[i] as char).is_whitespace() && bytes[i] != b'(' && bytes[i] != b')' { i += 1; } i
}

/// Skip one s-expression using the shared core scanner (handles strings,
/// comments, char literals, vectors). On malformed input (e.g. a stray `)` or
/// EOF) the position is left unchanged, so callers must guard against
/// no-progress when looping.
fn skip_sexp(bytes: &[u8], i: usize) -> usize {
    skip_sexp_d(bytes, i, Dialect::Generic)
}
fn skip_sexp_d(bytes: &[u8], i: usize, d: Dialect) -> usize {
    lisp_sitter_core::sexp_reader::skip_sexp_in(bytes, i, d).unwrap_or(i)
}

/// Try `plugin.find_sexp_in` (tree-sitter, skips strings/comments via AST),
/// falling back to the character scanner only when tree-sitter is unavailable.
fn find_sexp(plugin: &dyn LanguagePlugin, ft: &str, pat: &str, d: Dialect) -> Option<(usize, usize)> {
    match plugin.find_sexp_in(ft, pat) {
        Some(result) => result,          // tree-sitter search completed — trust result
        None => find_sexp_char(ft, pat, d), // parse unavailable — use character scanner
    }
}

/// Character-level fallback for `find_sexp`. Skips strings, line comments,
/// block comments, and char literals, then checks sexp boundaries.
fn find_sexp_char(ft: &str, pat: &str, d: Dialect) -> Option<(usize, usize)> {
    use lisp_sitter_core::sexp_reader::{skip_block_comment, skip_line_comment, skip_sexp_in, skip_string};
    let b = ft.as_bytes(); let pb = pat.as_bytes(); let mut i = 0;
    while i < b.len() {
        i = match b[i] {
            b'"' => skip_string(b, i).unwrap_or(b.len()),
            b';' => skip_line_comment(b, i).unwrap_or(b.len()),
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => skip_block_comment(b, i).unwrap_or(b.len()),
            b'?' if d == Dialect::Elisp => skip_sexp_in(b, i, d).unwrap_or(i + 1),
            _ => {
                if i + pb.len() <= b.len() && &b[i..i + pb.len()] == pb {
                    let n = i + pb.len();
                    let po = i == 0 || b[i - 1].is_ascii_whitespace() || b[i - 1] == b'(';
                    let no = n >= b.len() || b[n].is_ascii_whitespace() || b[n] == b')' || b[n] == b'(';
                    if po && no { return Some((i, n)); }
                }
                i + 1
            }
        };
    }
    None
}

/// Skip past leading docstrings (string literals) and Common Lisp `(declare …)` forms
/// so that `body_range` returns the range of the actual executable body.
fn skip_preamble(b: &[u8], mut pos: usize) -> usize {
    loop {
        pos = skip_sp(b, pos);
        if pos >= b.len() { break; }
        if b[pos] == b'"' {
            pos = skip_sexp(b, pos);
        } else if b[pos] == b'(' {
            let inner = skip_sp(b, pos + 1);
            let kw_end = skip_sym(b, inner);
            if kw_end > inner && &b[inner..kw_end] == b"declare" {
                pos = skip_sexp(b, pos);
            } else { break; }
        } else { break; }
    }
    pos
}

/// Find the body byte range within a form, skipping head, name, qualifiers,
/// param list, and any preamble. Uses the plugin's tree-sitter analysis when
/// available; falls back to the character-level scanner otherwise.
fn body_range(plugin: &dyn LanguagePlugin, ft: &str) -> Result<(usize, usize), Error> {
    if let Some((bs, be)) = plugin.form_body_range(ft) {
        if bs <= be && be <= ft.len() { return Ok((bs, be)); }
    }
    body_range_char(ft)
}

fn body_range_char(ft: &str) -> Result<(usize, usize), Error> {
    let b = ft.as_bytes(); let mut pos = 0;
    if pos >= b.len() || b[pos] != b'(' { return Err(Error::Message("form must start with (".into())); }
    pos += 1; pos = skip_sp(b, pos); pos = skip_sym(b, pos); pos = skip_sp(b, pos);
    pos = skip_sexp(b, pos); pos = skip_sp(b, pos); pos = skip_sexp(b, pos); pos = skip_sp(b, pos);
    pos = skip_preamble(b, pos);
    let bs = pos; if b.last() != Some(&b')') { return Err(Error::Message("form must end with )".into())); }
    let be = ft.len() - 1; if bs > be { return Err(Error::Message("no body to wrap".into())); }
    Ok((bs, be))
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
fn replace_head_symbol(plugin: &dyn LanguagePlugin, c: &str, old: &str, new: &str, refs: RefsMode) -> String {
    let mut positions: Vec<(usize, usize)> = plugin.find_symbol_refs(c, old)
        .into_iter()
        .filter(|r| match r.kind {
            RefKind::CallHead   => true,
            RefKind::SharpQuote => refs != RefsMode::HeadOnly,
            RefKind::Quote      => refs == RefsMode::AllRefs,
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

pub fn rename(reg: &Registry, path: &str, old: &str, new: &str, refs: RefsMode) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?;
    rename_content(p, &c, old, new, refs)
}

/// Rename the definition header of `old` plus its references within one file's
/// `content`. Shared by single-file [`rename`] and the per-file pass of
/// [`rename_project`].
fn rename_content(p: &dyn LanguagePlugin, c: &str, old: &str, new: &str, refs: RefsMode) -> Result<String, Error> {
    let u = replace_node_header(p, c, old, new)?;
    let u = replace_head_symbol(p, &u, old, new, refs);
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "rename".into(), detail: d }, o => o, })?;
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
pub fn rename_project(reg: &Registry, paths: &[String], old: &str, new: &str, refs: RefsMode) -> Result<Vec<(String, String)>, Error> {
    let mut changed = Vec::new();
    let mut found_def = false;
    for path in paths {
        let Ok(c) = ops_read(path) else { continue };
        let Ok(p) = crate::ops::resolve_plugin(reg, path, None) else { continue };
        let u = if p.node_bounds(&c, old).is_ok() {
            found_def = true;
            rename_content(p, &c, old, new, refs)?
        } else {
            if !touches_symbol(p, &c, old, refs) { continue; }
            let u = replace_head_symbol(p, &c, old, new, refs);
            p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "rename".into(), detail: d }, o => o, })?;
            u
        };
        if u != c { changed.push((path.clone(), u)); }
    }
    if !found_def { return Err(Error::FormNotFound(old.to_string())); }
    Ok(changed)
}

fn replace_node_header(p: &dyn lisp_sitter_core::LanguagePlugin, c: &str, old: &str, new: &str) -> Result<String, Error> {
    let ft = get_form_text(p, c, old)?;
    let renamed = p.form_rename_name(ft, old, new)
        .unwrap_or_else(|| replace_name_in_form_char(ft, old, new));
    replace_node(p, c, old, &renamed)
}

/// Fallback character-level rename of the definition name.
fn replace_name_in_form_char(t: &str, old: &str, new: &str) -> String {
    let s = t.trim(); if !s.starts_with('(') { return t.to_string(); }
    let a = &s[1..].trim_start(); let he = a.find(|c: char| c.is_whitespace()).unwrap_or(0);
    if he == 0 { return t.to_string(); }
    let ah = &a[he..].trim_start();
    if let Some(inner) = ah.strip_prefix('(') { let ne = inner.find(|c: char| c.is_whitespace() || c == ')').unwrap_or(inner.len());
        if &inner[..ne] == old { return format!("({} ({}{}", &a[..he], new, &inner[ne..]); } }
    else { let ne = ah.find(|c: char| c.is_whitespace()).unwrap_or(ah.len());
        if &ah[..ne] == old { return format!("({} {}{}", &a[..he], new, &ah[ne..]); } }
    t.to_string()
}

// ── remove ─────────────────────────────────────────────────────

fn remove_form_content(p: &dyn LanguagePlugin, c: &str, sym: &str, keep: bool) -> Result<String, Error> {
    ensure_source_editable(p, c)?;
    let (s, e) = p.node_bounds(c, sym)?;
    let rs = (0..s).rev().find(|&i| !c.as_bytes()[i].is_ascii_whitespace()).map(|i| i + 1).unwrap_or(s);
    let mut u = String::with_capacity(c.len()); u.push_str(&c[..rs]); if rs < s { u.push('\n'); } u.push_str(&c[e..]);
    if !keep {
        // Stub call sites with a dialect-appropriate no-op.
        // `ignore` is valid elisp; `values` is valid CL and Scheme (returns no values).
        let stub = if p.id() == "elisp" { "ignore" } else { "values" };
        u = replace_head_symbol(p, &u, sym, stub, RefsMode::HeadOnly);
    }
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "remove".into(), detail: d }, o => o, })?; Ok(u)
}

pub fn remove_form(reg: &Registry, path: &str, sym: &str, keep: bool) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?;
    remove_form_content(p, &c, sym, keep)
}

// ── move ───────────────────────────────────────────────────────

pub fn move_form(reg: &Registry, path: &str, sym: &str, after: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?;
    ensure_source_editable(p, &c)?;
    let ft = get_form_text(p, &c, sym)?.to_string();
    let removed = remove_form_content(p, &c, sym, true)?;
    let ins = insert_after(p, &removed, after, ft.trim())?;
    p.check_file(&ins).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "move".into(), detail: d }, o => o, })?; Ok(ins)
}

// ── substitute ─────────────────────────────────────────────────

pub fn substitute(reg: &Registry, path: &str, sym: &str, pat: &str, rep: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?; let ft = get_form_text(p, &c, sym)?;
    let (s, e) = find_sexp(p, ft, pat, dialect_of(p)).ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
    let nf = format!("{}{}{}", &ft[..s], rep, &ft[e..]); let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "substitute".into(), detail: d }, o => o, })?; Ok(u)
}

// ── extract ────────────────────────────────────────────────────

pub fn extract(reg: &Registry, path: &str, sym: &str, pat: &str, name: &str, params: &[&str]) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?; let ft = get_form_text(p, &c, sym)?;
    let (s, e) = find_sexp(p, ft, pat, dialect_of(p)).ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
    let ex = &ft[s..e]; let fv = if params.is_empty() { detect_syms(ex, p) } else { params.to_vec() };
    let ps = if fv.is_empty() { "()".to_string() } else { format!("({})", fv.join(" ")) };
    let nd = if p.id() == "scheme" { format!("(define ({name} {ps})\n  {ex})\n") } else { format!("(defun {name} {ps}\n  {ex})\n") };
    let call = if fv.is_empty() { format!("({name})") } else { format!("({} {})", name, fv.join(" ")) };
    let uf = format!("{}{}{}", &ft[..s], &call, &ft[e..]); let as_ = replace_node(p, &c, sym, &uf)?;
    let p2 = crate::ops::resolve_plugin(reg, path, None)?; let ins = insert_after(p2, &as_, sym, &nd)?;
    p2.check_file(&ins).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "extract".into(), detail: d }, o => o, })?; Ok(ins)
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
        if i >= b.len() { break; }
        match b[i] {
            b'(' | b')' => { i += 1; }
            b'"' => { i += 1; while i < b.len() && b[i] != b'"' { if b[i] == b'\\' { i += 2; continue; } i += 1; } i += 1; }
            b'\'' | b'`' | b',' => { i += 1; }
            b';' => { while i < b.len() && b[i] != b'\n' { i += 1; } }
            _ => {
                let s = i; i = skip_sym(b, i); let sym = &sexp[s..i];
                if !sym.is_empty() && !sym.starts_with(|c: char| c.is_ascii_digit())
                    && !matches!(sym, "nil" | "t" | "#t" | "#f" | "true" | "false") {
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

pub fn wrap_body(reg: &Registry, path: &str, sym: &str, wrapper: &str, args: &[(&str, &str)]) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?; let ft = get_form_text(p, &c, sym)?;
    let b = body_range(p, ft)?; let nf = format!("{}{}{}", &ft[..b.0], make_wrapper(wrapper, args, &ft[b.0..b.1])?, &ft[b.1..]);
    replace_node(p, &c, sym, &nf)
}

fn make_wrapper(w: &str, a: &[(&str, &str)], body: &str) -> Result<String, Error> {
    let b = body.trim();
    match w { "progn" => Ok(format!("(progn\n  {})", b.replace('\n', "\n  "))),
        "let" => { let bind = a.iter().find(|(k, _)| *k == "bindings").map(|(_, v)| *v).unwrap_or("()");
            Ok(format!("(let {}\n  {})", bind, b.replace('\n', "\n  "))) }
        "if" => { let cond = a.iter().find(|(k, _)| *k == "condition").map(|(_, v)| *v).unwrap_or("t");
            Ok(format!("(if {cond}\n    {}\n  nil)", b)) }
        o => Err(Error::Message(format!("unknown wrapper: {o}"))) }
}

// ── instrument ─────────────────────────────────────────────────

pub fn instrument(reg: &Registry, path: &str, sym: &str, with: Option<&str>, at: Option<&str>, wrap: Option<&str>) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?; let ft = get_form_text(p, &c, sym)?;
    let nf = if let Some(tf) = with { let b = body_range(p, ft)?; format!("{}{}{}", &ft[..b.0], instr_body(&ft[b.0..b.1], tf), &ft[b.1..]) }
        else if let (Some(pat), Some(wrp)) = (at, wrap) { let (s, e) = find_sexp(p, ft, pat, dialect_of(p)).ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
            format!("{}{}{}", &ft[..s], &wrp.replace("<form>", pat), &ft[e..]) }
        else { return Err(Error::Message("provide --with or --at --wrap".into())); };
    let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "instrument".into(), detail: d }, o => o, })?; Ok(u)
}

fn instr_body(body: &str, trace: &str) -> String {
    let b = body.as_bytes(); let mut out = String::new(); let mut i = 0; let mut first = true;
    loop { while i < b.len() && b[i].is_ascii_whitespace() { i += 1; } if i >= b.len() { break; }
        let n = skip_sexp(b, i); if n <= i { break; } let f = body[i..n].trim();
        if !f.is_empty() { if !first { out.push('\n'); } out.push_str(&format!("(progn\n  {}\n  {})", trace, f)); first = false; }
        i = n; }
    out
}

// ── flatten ────────────────────────────────────────────────────

/// Byte ranges of the elements inside a form `(head e1 e2 …)`, relative to `ft`.
fn split_elements(ft: &str, d: Dialect) -> Vec<(usize, usize)> {
    let b = ft.as_bytes();
    let open = match ft.find('(') { Some(o) => o + 1, None => return Vec::new() };
    let close = ft.rfind(')').unwrap_or(ft.len());
    let mut i = open;
    let mut elems = Vec::new();
    while i < close {
        i = skip_sp(b, i);
        if i >= close { break; }
        let s = i;
        let e = skip_sexp_d(b, i, d).min(close);
        if e <= s { break; }
        elems.push((s, e));
        i = e;
    }
    elems
}

/// Parse a function definition into (param names, single body expression).
/// Uses the plugin's tree-sitter analysis when available; falls back to the
/// character-level scanner. Multi-form bodies are wrapped with `progn`/`begin`.
fn def_params_and_body(plugin: &dyn LanguagePlugin, ft: &str, d: Dialect) -> Option<(Vec<String>, String)> {
    if let Some((params, body_text)) = plugin.form_params_and_body(ft) {
        // body_text may be a single expression or multiple; wrap if needed.
        let body = wrap_multi_body(&body_text, d);
        return Some((params, body));
    }
    def_params_and_body_char(ft, d)
}

/// Wrap multiple body forms into a single expression if needed.
fn wrap_multi_body(body_text: &str, d: Dialect) -> String {
    // Count top-level forms in the body text.
    let b = body_text.as_bytes();
    let mut i = 0; let mut count = 0;
    while i < b.len() {
        i = lisp_sitter_core::sexp_reader::skip_whitespace_and_comments(b, i);
        if i >= b.len() { break; }
        match lisp_sitter_core::sexp_reader::skip_sexp_in(b, i, d) {
            Ok(next) => { count += 1; i = next; }
            Err(_) => break,
        }
    }
    if count <= 1 { body_text.trim().to_string() }
    else {
        let kw = if d == Dialect::Generic { "begin" } else { "progn" };
        format!("({kw} {})", body_text.trim())
    }
}

/// Character-level fallback for `def_params_and_body`.
fn def_params_and_body_char(ft: &str, d: Dialect) -> Option<(Vec<String>, String)> {
    let elems = split_elements(ft, d);
    if elems.len() < 3 { return None; }
    let head = &ft[elems[0].0..elems[0].1];
    let close = ft.rfind(')').unwrap_or(ft.len());

    let (params, body_idx) = if head == "define"
        && ft[elems[1].0..elems[1].1].starts_with('(')
    {
        // Scheme curried define: (define (name p…) body…)
        let sig = &ft[elems[1].0..elems[1].1];
        let sig_elems = split_elements(sig, d);
        if sig_elems.is_empty() { return None; }
        let params = sig_elems.iter().skip(1).map(|(s, e)| sig[*s..*e].to_string()).collect();
        (params, 2)
    } else {
        // (head name (args) body…)
        let arglist = &ft[elems[2].0..elems[2].1];
        if !arglist.starts_with('(') { return None; }
        let params = split_elements(arglist, d).iter().map(|(s, e)| arglist[*s..*e].to_string()).collect();
        (params, 3)
    };

    if elems.len() <= body_idx { return None; }
    let body_text = ft[elems[body_idx].0..close].trim().to_string();
    if body_text.is_empty() { return None; }
    let body = if elems.len() - body_idx > 1 {
        let kw = if d == Dialect::Generic && head == "define" { "begin" } else { "progn" };
        format!("({kw} {body_text})")
    } else {
        body_text
    };
    Some((params, body))
}

/// Replace every whole-symbol occurrence of `name` with `repl`, skipping
/// strings, comments and char literals.
fn substitute_symbol(text: &str, name: &str, repl: &str, d: Dialect) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => { let e = lisp_sitter_core::sexp_reader::skip_string(b, i).unwrap_or(b.len()); out.push_str(&text[i..e]); i = e; }
            b';' => { let e = lisp_sitter_core::sexp_reader::skip_line_comment(b, i).unwrap_or(b.len()); out.push_str(&text[i..e]); i = e; }
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => { let e = lisp_sitter_core::sexp_reader::skip_block_comment(b, i).unwrap_or(b.len()); out.push_str(&text[i..e]); i = e; }
            b'#' if i + 1 < b.len() && b[i + 1] == b'\\' => { let e = skip_sexp_d(b, i, d); out.push_str(&text[i..e]); i = e; }
            b'?' if d == Dialect::Elisp => { let e = skip_sexp_d(b, i, d); out.push_str(&text[i..e]); i = e; }
            b'(' | b')' | b'\'' | b'`' | b',' => { out.push(b[i] as char); i += 1; }
            c if c.is_ascii_whitespace() => { out.push(c as char); i += 1; }
            _ => {
                let s = i;
                let e = skip_sym(b, i).max(s + 1);
                let tok = &text[s..e];
                if tok == name { out.push_str(repl); } else { out.push_str(tok); }
                i = e;
            }
        }
    }
    out
}

/// Inline every genuine call site of `sym` in `content` by substituting
/// arguments into `body`. Uses the plugin's tree-sitter-backed `find_symbol_refs`
/// so that strings, comments, char literals, and let-binding variable positions
/// are excluded automatically. `def_bounds` excludes the definition form itself
/// (e.g. Scheme curried `(define (sym …) …)` which looks like a call).
/// Replacement is applied back-to-front.
fn inline_calls(plugin: &dyn LanguagePlugin, content: &str, sym: &str, params: &[String], body: &str, d: Dialect, def_bounds: Option<(usize, usize)>) -> Result<String, Error> {
    let b = content.as_bytes();

    // Collect all genuine call positions, sorted back-to-front.
    let mut call_sites: Vec<usize> = plugin.find_symbol_refs(content, sym)
        .into_iter()
        .filter(|r| r.kind == RefKind::CallHead)
        .filter(|r| !def_bounds.is_some_and(|(ds, de)| r.form_start >= ds && r.form_start < de))
        .map(|r| r.form_start)
        .collect();
    call_sites.sort_unstable_by_key(|a| std::cmp::Reverse(*a));

    let mut result = content.to_string();
    for form_start in call_sites {
        let call_end = skip_sexp_d(b, form_start, d);
        if call_end <= form_start || call_end > result.len() { continue; }
        let call_text = &result[form_start..call_end];
        let args: Vec<String> = split_elements(call_text, d)
            .iter().skip(1).map(|(s, e)| call_text[*s..*e].to_string()).collect();
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

pub fn flatten(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    ensure_source_editable(p, &c)?;
    let d = dialect_of(p);
    let ft = get_form_text(p, &c, sym)?.to_string();

    let (params, body) = def_params_and_body(p, &ft, d).ok_or_else(|| {
        Error::Message(format!("flatten: `{sym}` is not a flattenable function definition"))
    })?;
    if params.iter().any(|pn| pn.starts_with('&') || pn.starts_with('(') || pn.is_empty()) {
        return Err(Error::Message(
            "flatten supports only simple positional parameters (no &rest/&optional/&key/destructuring)".into(),
        ));
    }
    if p.find_symbol_refs(&body, sym).iter().any(|r| r.kind == RefKind::CallHead) {
        return Err(Error::Message(format!("flatten: `{sym}` is recursive; cannot inline")));
    }

    // Exclude the definition form itself from call-site inlining (a Scheme curried
    // define `(define (sym args…) …)` has sym in head position inside the signature).
    let def_bounds = p.node_bounds(&c, sym).ok();
    // Inline call sites first, then remove the (now-unreferenced) definition.
    let inlined = inline_calls(p, &c, sym, &params, &body, d, def_bounds)?;
    let (s, e) = p.node_bounds(&inlined, sym)?;
    let rs = (0..s)
        .rev()
        .find(|&i| !inlined.as_bytes()[i].is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(s);
    let mut u = String::with_capacity(inlined.len());
    u.push_str(&inlined[..rs]);
    if rs < s { u.push('\n'); }
    u.push_str(&inlined[e..]);

    p.check_file(&u).map_err(|e| match e {
        Error::Syntax(detail) => Error::SyntaxAfterEdit { operation: "flatten".into(), detail },
        o => o,
    })?;
    Ok(u)
}

// ── splice ─────────────────────────────────────────────────────

/// Remove the outer parentheses of a sub-expression, splicing its children
/// into the parent in its place.  Example: `(progn (a) (b))` → `(a) (b)`.
pub fn splice(reg: &Registry, path: &str, sym: &str, pat: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let d = dialect_of(p);
    let (s, e) = find_sexp(p, ft, pat, d)
        .ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
    let b = ft.as_bytes();
    if b.get(s) != Some(&b'(') || b.get(e.saturating_sub(1)) != Some(&b')') {
        return Err(Error::Message("splice: pattern must be a parenthesised list".into()));
    }
    // Skip the head element (e.g. `progn` in `(progn A B)`) so the body is
    // elevated, not the bare head symbol.
    let after_open = s + 1;
    let ib = &b[after_open..e - 1];
    let mut head_end = 0;
    while head_end < ib.len() && ib[head_end].is_ascii_whitespace() { head_end += 1; }
    head_end = skip_sexp_d(ib, head_end, d).min(ib.len());
    while head_end < ib.len() && ib[head_end].is_ascii_whitespace() { head_end += 1; }
    let inner = ft[after_open + head_end..e - 1].trim();
    let nf = format!("{}{}{}", &ft[..s], inner, &ft[e..]);
    let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e {
        Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "splice".into(), detail: d },
        o => o,
    })?;
    Ok(u)
}

// ── raise ──────────────────────────────────────────────────────

/// Replace the direct enclosing list of a sub-expression with just that
/// sub-expression.  Example: `(if cond (bar x) nil)` raise `(bar x)` → `(bar x)`.
pub fn raise(reg: &Registry, path: &str, sym: &str, pat: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = crate::ops::resolve_plugin(reg, path, None)?;
    let ft = get_form_text(p, &c, sym)?;
    let d = dialect_of(p);
    let (s, e) = find_sexp(p, ft, pat, d)
        .ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
    let (ps, pe) = find_enclosing_sexp(ft, s, d)
        .ok_or_else(|| Error::Message("raise: pattern has no enclosing form to replace".into()))?;
    let raised = ft[s..e].to_string();
    let nf = format!("{}{}{}", &ft[..ps], raised, &ft[pe..]);
    let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e {
        Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "raise".into(), detail: d },
        o => o,
    })?;
    Ok(u)
}

/// Walk `text` byte-by-byte up to `inner_start`, maintaining a paren stack.
/// Returns the byte range `[start, end)` of the innermost `(...)` that directly
/// contains the position `inner_start`.  Returns `None` when `inner_start` is
/// already at the top level (depth 0).
fn find_enclosing_sexp(text: &str, inner_start: usize, d: Dialect) -> Option<(usize, usize)> {
    use lisp_sitter_core::sexp_reader::{
        skip_atom_in, skip_block_comment, skip_line_comment, skip_sexp_in, skip_string,
    };
    let b = text.as_bytes();
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < inner_start {
        while i < inner_start && i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= inner_start || i >= b.len() {
            break;
        }
        match b[i] {
            b'(' => { stack.push(i); i += 1; }
            b')' => { stack.pop(); i += 1; }
            b'"' => { i = skip_string(b, i).unwrap_or(b.len()); }
            b';' => { i = skip_line_comment(b, i).unwrap_or(b.len()); }
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => {
                i = skip_block_comment(b, i).unwrap_or(b.len());
            }
            b'#' if i + 1 < b.len() && b[i + 1] == b';' => {
                i = skip_sexp_in(b, i + 2, d).unwrap_or(b.len());
            }
            b'\'' | b'`' => { i += 1; }
            b',' => { i += if i + 1 < b.len() && b[i + 1] == b'@' { 2 } else { 1 }; }
            _ => { i = skip_atom_in(b, i, d).unwrap_or(i + 1); }
        }
    }
    let parent_start = *stack.last()?;
    let parent_end = skip_sexp_in(b, parent_start, d).ok()?;
    Some((parent_start, parent_end))
}

// ── convert-let ────────────────────────────────────────────────

pub fn convert_let(reg: &Registry, path: &str, sym: &str, target: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = crate::ops::resolve_plugin(reg, path, None)?; let ft = get_form_text(p, &c, sym)?;
    let (from_space, from_nl, to) = if target == "let*" {
        ("(let ", "(let\n", "(let* ")
    } else {
        ("(let* ", "(let*\n", "(let ")
    };
    let nf = if ft.contains(from_space) {
        ft.replacen(from_space, to, 1)
    } else if ft.contains(from_nl) {
        ft.replacen(from_nl, &format!("{}\n", to.trim_end()), 1)
    } else {
        return Err(Error::Message(format!(
            "form `{sym}` does not contain `{}`; cannot convert to {target}", from_space.trim()
        )));
    };
    let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "convert-let".into(), detail: d }, o => o, })?; Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_registry;
    use lisp_sitter_core::edit::replace_node;
    use lisp_sitter_core::Error;

    fn tmp_file(name: &str, content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lisp-sitter-transform-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.el");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn test_rename() {
        let reg = default_registry();
        let (dir, path) = tmp_file("rename",
            "(defun foo ()\n  1)\n\n(defun bar ()\n  (foo))\n");
        let result = rename(&reg, path.to_str().unwrap(), "foo", "baz", RefsMode::HeadAndSharp).unwrap();
        assert!(result.contains("(defun baz ()"));
        assert!(result.contains("(baz)"));
        assert!(result.contains("(defun bar ()"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_form_malformed_source_refused() {
        let reg = default_registry();
        let (dir, path) = tmp_file("remove_malformed",
            "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n");
        let result = remove_form(&reg, path.to_str().unwrap(), "b", true);
        assert!(matches!(result, Err(Error::MalformedSource(_))), "remove should refuse malformed source: {:?}", result);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("(defun c () 3)"), "form c must survive: {content}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_replace_malformed_source_refused() {
        let reg = default_registry();
        let (dir, path) = tmp_file("replace_malformed",
            "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n");
        let result = replace_node(
            reg.plugin_for_path(path.to_str().unwrap()).unwrap(),
            std::fs::read_to_string(&path).unwrap().as_str(),
            "b",
            "(defun b (x) (+ x 1))",
        );
        assert!(matches!(result, Err(Error::MalformedSource(_))), "replace should refuse malformed source: {:?}", result);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("(defun c () 3)"), "form c must survive: {content}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_form() {
        let reg = default_registry();
        let (dir, path) = tmp_file("remove",
            "(defun foo ()\n  1)\n\n(defun bar ()\n  (foo))\n");
        // remove without keeping calls — foo body replaced with (ignore)
        let result = remove_form(&reg, path.to_str().unwrap(), "foo", false).unwrap();
        assert!(!result.contains("(defun foo"));
        assert!(result.contains("(ignore)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_form_keep_calls() {
        let reg = default_registry();
        let (dir, path) = tmp_file("remove_keep",
            "(defun foo ()\n  1)\n\n(defun bar ()\n  (foo))\n");
        let result = remove_form(&reg, path.to_str().unwrap(), "foo", true).unwrap();
        assert!(!result.contains("(defun foo"));
        assert!(result.contains("(foo)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_move_form() {
        let reg = default_registry();
        let (dir, path) = tmp_file("move",
            "(defun a ()\n  1)\n\n(defun b ()\n  2)\n");
        let result = move_form(&reg, path.to_str().unwrap(), "a", "b").unwrap();
        let a_pos = result.find("(defun a").unwrap();
        let b_pos = result.find("(defun b").unwrap();
        assert!(b_pos < a_pos, "a should be moved after b");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_move_form_malformed_source_refused() {
        let reg = default_registry();
        let (dir, path) = tmp_file("move_malformed",
            "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n");
        let result = move_form(&reg, path.to_str().unwrap(), "a", "c");
        assert!(matches!(result, Err(Error::MalformedSource(_))), "move should refuse malformed source: {:?}", result);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_substitute() {
        let reg = default_registry();
        let (dir, path) = tmp_file("subst",
            "(defun foo (x)\n  (+ x 1))\n");
        let result = substitute(&reg, path.to_str().unwrap(), "foo", "(+ x 1)", "(* x 2)").unwrap();
        assert!(result.contains("(* x 2)"));
        assert!(!result.contains("(+ x 1)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract() {
        let reg = default_registry();
        let (dir, path) = tmp_file("extract",
            "(defun foo (x)\n  (+ x 1))\n");
        // extract inserts the new helper after the original definition
        let result = extract(&reg, path.to_str().unwrap(), "foo", "(+ x 1)", "add1", &["x"]);
        assert!(result.is_ok(), "extract failed: {:?}", result.err());
        let got = result.unwrap();
        assert!(got.contains("defun add1"), "should define add1: {got}");
        assert!(got.contains("(add1 x)"), "should call add1: {got}");
        // The original (+ x 1) is now only in the extracted definition
        let foo_line: Vec<&str> = got.lines().filter(|l| l.trim().starts_with("(defun foo")).collect();
        assert_eq!(foo_line.len(), 1, "foo defined once: {got}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wrap_body_progn() {
        let reg = default_registry();
        let (dir, path) = tmp_file("wrap_progn",
            "(defun foo ()\n  (+ 1 2))\n");
        let result = wrap_body(&reg, path.to_str().unwrap(), "foo", "progn", &[]).unwrap();
        assert!(result.contains("(progn"));
        assert!(result.contains("(+ 1 2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wrap_body_let() {
        let reg = default_registry();
        let (dir, path) = tmp_file("wrap_let",
            "(defun foo ()\n  (+ 1 2))\n");
        let result = wrap_body(&reg, path.to_str().unwrap(), "foo", "let", &[("bindings", "((x 1))")]).unwrap();
        assert!(result.contains("(let ((x 1))"));
        assert!(result.contains("(+ 1 2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wrap_body_if() {
        let reg = default_registry();
        let (dir, path) = tmp_file("wrap_if",
            "(defun foo ()\n  (+ 1 2))\n");
        let result = wrap_body(&reg, path.to_str().unwrap(), "foo", "if", &[("condition", "(> x 0)")]).unwrap();
        assert!(result.contains("(if (> x 0)"));
        assert!(result.contains("(+ 1 2)"));
        assert!(result.contains("nil"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_instrument_with_trace() {
        let reg = default_registry();
        let (dir, path) = tmp_file("instr_with",
            "(defun foo ()\n  (+ 1 2))\n");
        let result = instrument(&reg, path.to_str().unwrap(), "foo", Some("(message \"trace\")"), None, None).unwrap();
        assert!(result.contains("(progn"));
        assert!(result.contains("(message \"trace\")"));
        assert!(result.contains("(+ 1 2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_instrument_wrap() {
        let reg = default_registry();
        let (dir, path) = tmp_file("instr_wrap",
            "(defun foo ()\n  (+ 1 2))\n");
        let result = instrument(&reg, path.to_str().unwrap(), "foo", None, Some("(+ 1 2)"), Some("(list <form>)")).unwrap();
        assert!(result.contains("(list (+ 1 2))"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_flatten() {
        let reg = default_registry();
        let (dir, path) = tmp_file("flatten",
            "(defun add1 (x)\n  (+ x 1))\n\n(defun foo ()\n  (add1 2))\n");
        let result = flatten(&reg, path.to_str().unwrap(), "add1").unwrap();
        // The definition is removed AND the call site is inlined with the
        // argument substituted: (add1 2) -> (+ 2 1).
        assert!(!result.contains("(defun add1"), "definition should be gone: {result}");
        assert!(result.contains("(+ 2 1)"), "call should be inlined: {result}");
        assert!(!result.contains("(add1 2)"), "call should not remain: {result}");
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
        let (dir, path) = tmp_file("flatten_arity",
            "(defun add (a b)\n  (+ a b))\n\n(defun use ()\n  (add 1))\n");
        let result = flatten(&reg, path.to_str().unwrap(), "add");
        assert!(result.is_err(), "arity mismatch should error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_splice_progn() {
        let reg = default_registry();
        let (dir, path) = tmp_file("splice_progn",
            "(defun foo (x)\n  (when condition (progn (do-a x) (do-b x))))\n");
        let result = splice(&reg, path.to_str().unwrap(), "foo", "(progn (do-a x) (do-b x))").unwrap();
        assert!(!result.contains("progn"), "progn should be dissolved: {result}");
        assert!(result.contains("(do-a x)"), "do-a must survive: {result}");
        assert!(result.contains("(do-b x)"), "do-b must survive: {result}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_splice_rejects_atom() {
        let reg = default_registry();
        let (dir, path) = tmp_file("splice_atom",
            "(defun foo (x)\n  (+ x 1))\n");
        let result = splice(&reg, path.to_str().unwrap(), "foo", "x");
        assert!(result.is_err(), "splicing an atom should error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_raise_replaces_enclosing() {
        let reg = default_registry();
        let (dir, path) = tmp_file("raise_if",
            "(defun foo (x)\n  (if condition (bar x) nil))\n");
        let result = raise(&reg, path.to_str().unwrap(), "foo", "(bar x)").unwrap();
        assert!(result.contains("(bar x)"), "bar must survive: {result}");
        assert!(!result.contains("(if condition"), "if should be replaced: {result}");
        assert!(!result.contains("nil"), "else branch should be gone: {result}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_raise_pattern_not_found() {
        let reg = default_registry();
        let (dir, path) = tmp_file("raise_miss",
            "(defun foo (x)\n  (+ x 1))\n");
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
        let (dir, path) = tmp_file("conv_let",
            "(defun foo ()\n  (let ((x 1) (y 2)) (+ x y)))\n");
        let result = convert_let(&reg, path.to_str().unwrap(), "foo", "let*").unwrap();
        assert!(result.contains("(let*"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_convert_let_star_to_let() {
        let reg = default_registry();
        let (dir, path) = tmp_file("conv_let*",
            "(defun foo ()\n  (let* ((x 1) (y 2)) (+ x y)))\n");
        let result = convert_let(&reg, path.to_str().unwrap(), "foo", "let").unwrap();
        assert!(result.contains("(let "));
        assert!(!result.contains("(let*"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- unit tests for internal helpers --------------------------------

    #[test]
    fn test_find_sexp_basic() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        assert_eq!(find_sexp(p, "(+ x 1)", "(+ x 1)", Dialect::Generic), Some((0, 7)));
        assert_eq!(find_sexp(p, "calls (foo) and (bar)", "(foo)", Dialect::Generic), Some((6, 11)));
    }

    #[test]
    fn test_find_sexp_not_found() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        assert_eq!(find_sexp(p, "(defun foo ())", "(bar)", Dialect::Generic), None);
    }

    #[test]
    fn test_find_sexp_skips_line_comment() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  ; (bar x) in comment\n  (bar x))";
        assert_eq!(find_sexp(p, ft, "(bar x)", Dialect::Generic),
            Some((ft.rfind("(bar x)").unwrap(), ft.rfind("(bar x)").unwrap() + 7)));
    }

    #[test]
    fn test_body_range() {
        let reg = default_registry(); let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  (+ x 1))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_skips_docstring() {
        let reg = default_registry(); let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  \"docstring\"\n  (+ x 1))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_skips_declare() {
        let reg = default_registry(); let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(defun foo (x)\n  (declare (type integer x))\n  (+ x 1))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_skips_docstring_and_declare() {
        let reg = default_registry(); let p = reg.plugin_for_id("elisp").unwrap();
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
        let reg = default_registry(); let p = reg.plugin_for_id("elisp").unwrap();
        let ft = "(cl-defmethod my-fn :before ((obj integer) x)\n  (+ obj x))";
        let range = body_range(p, ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ obj x)");
    }

    #[test]
    fn test_inline_calls_skips_let_binding_var() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let content = "(defun test () (let ((my-fn 42)) (my-fn 10)))";
        let result = inline_calls(p, content, "my-fn", &["x".to_string()], "(+ x 1)", Dialect::Elisp, None).unwrap();
        assert!(result.contains("(let ((my-fn 42))"), "binding spec must be untouched: {result}");
        assert!(result.contains("(+ 10 1)"), "call in body must be inlined: {result}");
    }

    #[test]
    fn test_inline_calls_inlines_binding_init() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let content = "(defun test () (let ((x (my-fn 42))) (my-fn x)))";
        let result = inline_calls(p, content, "my-fn", &["n".to_string()], "(+ n 1)", Dialect::Elisp, None).unwrap();
        assert!(result.contains("(let ((x (+ 42 1)))"), "init expression must be inlined: {result}");
        assert!(result.contains("(+ x 1)"), "call in body must be inlined: {result}");
    }

    #[test]
    fn test_replace_name_in_form_char() {
        let result = replace_name_in_form_char("(defun foo (x) (+ x 1))", "foo", "bar");
        assert_eq!(result, "(defun bar (x) (+ x 1))");
    }

    #[test]
    fn test_replace_name_in_form_no_opener() {
        assert_eq!(replace_name_in_form_char("just a string", "x", "y"), "just a string");
    }

    #[test]
    fn test_make_wrapper_progn() {
        let result = make_wrapper("progn", &[], "(+ 1 2)").unwrap();
        assert_eq!(result, "(progn\n  (+ 1 2))");
    }

    #[test]
    fn test_make_wrapper_let() {
        let result = make_wrapper("let", &[("bindings", "((x 1))")], "(+ x 1)").unwrap();
        assert_eq!(result, "(let ((x 1))\n  (+ x 1))");
    }

    #[test]
    fn test_make_wrapper_if() {
        let result = make_wrapper("if", &[("condition", "(> x 0)")], "(+ x 1)").unwrap();
        assert_eq!(result, "(if (> x 0)\n    (+ x 1)\n  nil)");
    }

    #[test]
    fn test_make_wrapper_unknown() {
        assert!(make_wrapper("unknown", &[], "body").is_err());
    }

    #[test]
    fn test_detect_syms() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let syms = detect_syms("(+ x 1)", p);
        // `+` appears only as call head → filtered out as function name
        assert!(!syms.contains(&"+"), "built-in function should be excluded: {syms:?}");
        assert!(!syms.contains(&"1"), "numbers should be excluded: {syms:?}");
        // `x` appears in value position → kept as free variable
        assert!(syms.contains(&"x"), "free variable x should be detected: {syms:?}");
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
        assert!(result.contains("\"call (foo) here\""), "string must be untouched: {result}");
        assert!(result.contains("; foo in comment"), "comment must be untouched: {result}");
    }

    #[test]
    fn test_replace_head_symbol_sharp_refs() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let c = "(foo)\n(add-hook 'h #'foo)\n(setq x 'foo)";
        let got = replace_head_symbol(p, c, "foo", "baz", RefsMode::HeadAndSharp);
        assert!(got.contains("(baz)"), "{got}");
        assert!(got.contains("#'baz"), "#' should be renamed: {got}");
        assert!(got.contains("'foo"), "plain 'foo should not be renamed without --refs: {got}");
    }

    #[test]
    fn test_replace_head_symbol_all_refs() {
        let reg = default_registry();
        let p = reg.plugin_for_id("elisp").unwrap();
        let c = "(foo)\n(add-hook 'h #'foo)\n(setq sym 'foo)";
        let got = replace_head_symbol(p, c, "foo", "baz", RefsMode::AllRefs);
        assert!(got.contains("(baz)"), "{got}");
        assert!(got.contains("#'baz"), "{got}");
        assert!(got.contains("'baz"), "plain 'foo should be renamed with --refs: {got}");
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
        let syms = detect_syms(r#"(some-fn "string with (parens)" ; comment
  x)"#, p);
        assert!(!syms.contains(&"+"), "non-existent + should not be in syms: {syms:?}");
        // some-fn appears only as a call head → filtered as function name
        assert!(!syms.contains(&"some-fn"), "call-head function should be excluded: {syms:?}");
        // x appears as a value → included
        assert!(syms.contains(&"x"), "free variable x should be detected: {syms:?}");
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
        assert!(!syms.contains(&"format"), "format (builtin) should be excluded: {syms:?}");
        // `x` appears in value position → included
        assert!(syms.contains(&"x"), "free variable x should be included: {syms:?}");
    }
}
