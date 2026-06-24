use lisp_sitter_core::edit::{get_form_text, insert_after, replace_node};
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

fn find_sexp(ft: &str, pat: &str) -> Option<(usize, usize)> {
    let b = ft.as_bytes(); let pb = pat.as_bytes(); let mut s = 0;
    loop { let pos = ft[s..].find(pat)?; let a = s + pos;
        let po = a == 0 || b[a-1].is_ascii_whitespace() || b[a-1] == b'(';
        let n = a + pb.len(); let no = n >= b.len() || b[n].is_ascii_whitespace() || b[n] == b')' || b[n] == b'(';
        if po && no { return Some((a, n)); } s = a + 1; }
}

fn body_range(ft: &str) -> Result<(usize, usize), Error> {
    let b = ft.as_bytes(); let mut pos = 0;
    if pos >= b.len() || b[pos] != b'(' { return Err(Error::Message("form must start with (".into())); }
    pos += 1; pos = skip_sp(b, pos); pos = skip_sym(b, pos); pos = skip_sp(b, pos);
    pos = skip_sexp(b, pos); pos = skip_sp(b, pos); pos = skip_sexp(b, pos); pos = skip_sp(b, pos);
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

/// Replace `old` with `new` where it appears as the head symbol of a list —
/// `(old …)` / `(old)` call sites — leaving strings, comments and char literals
/// untouched. `refs` controls which quoted/sharp-quoted references are renamed.
fn replace_head_symbol(c: &str, old: &str, new: &str, d: Dialect, refs: RefsMode) -> String {
    let b = c.as_bytes();
    let mut out = String::with_capacity(c.len() + new.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let e = lisp_sitter_core::sexp_reader::skip_string(b, i)
                    .unwrap_or(b.len());
                out.push_str(&c[i..e]);
                i = e;
            }
            b';' => {
                let e = lisp_sitter_core::sexp_reader::skip_line_comment(b, i)
                    .unwrap_or(b.len());
                out.push_str(&c[i..e]);
                i = e;
            }
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => {
                let e = lisp_sitter_core::sexp_reader::skip_block_comment(b, i)
                    .unwrap_or(b.len());
                out.push_str(&c[i..e]);
                i = e;
            }
            // #'old — sharp-quoted function reference (default)
            b'#' if (refs != RefsMode::HeadOnly) && i + 1 < b.len() && b[i + 1] == b'\'' => {
                out.push_str("#'");
                i += 2;
                let sym_end = skip_sym(b, i);
                if sym_end > i && &c[i..sym_end] == old {
                    out.push_str(new);
                    i = sym_end;
                }
            }
            // char literals — copy verbatim so an embedded paren isn't treated as a list
            b'#' if i + 1 < b.len() && b[i + 1] == b'\\' => {
                let e = skip_sexp_d(b, i, d);
                out.push_str(&c[i..e]);
                i = e;
            }
            b'?' if d == Dialect::Elisp => {
                let e = skip_sexp_d(b, i, d);
                out.push_str(&c[i..e]);
                i = e;
            }
            // 'old — quoted symbol reference (only with AllRefs; not for quoted lists '(…))
            b'\'' if refs == RefsMode::AllRefs && (i + 1 >= b.len() || b[i + 1] != b'(') => {
                out.push('\'');
                i += 1;
                let sym_end = skip_sym(b, i);
                if sym_end > i && &c[i..sym_end] == old {
                    out.push_str(new);
                    i = sym_end;
                }
            }
            b'(' => {
                out.push('(');
                i += 1;
                let ws = skip_sp(b, i);
                let sym_end = skip_sym(b, ws);
                if &c[ws..sym_end] == old && sym_end > ws {
                    out.push_str(&c[i..ws]); // preserve whitespace after (
                    out.push_str(new);
                    i = sym_end;
                }
            }
            _ => {
                let l = utf8_len(b[i]);
                let e = (i + l).min(b.len());
                out.push_str(&c[i..e]);
                i = e;
            }
        }
    }
    out
}

/// Byte length of the UTF-8 char whose lead byte is `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 { 1 } else if b >> 5 == 0b110 { 2 } else if b >> 4 == 0b1110 { 3 } else if b >> 3 == 0b11110 { 4 } else { 1 }
}

// ── rename ─────────────────────────────────────────────────────

pub fn rename(reg: &Registry, path: &str, old: &str, new: &str, refs: RefsMode) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?;
    let u = replace_node_header(p, &c, old, new)?;
    let u = replace_head_symbol(&u, old, new, dialect_of(p), refs);
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "rename".into(), detail: d }, o => o, })?; Ok(u)
}

fn replace_node_header(p: &dyn lisp_sitter_core::LanguagePlugin, c: &str, old: &str, new: &str) -> Result<String, Error> {
    replace_node(p, c, old, &replace_name_in_form(get_form_text(p, c, old)?, old, new))
}

fn replace_name_in_form(t: &str, old: &str, new: &str) -> String {
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

pub fn remove_form(reg: &Registry, path: &str, sym: &str, keep: bool) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let (s, e) = p.node_bounds(&c, sym)?;
    let rs = (0..s).rev().find(|&i| !c.as_bytes()[i].is_ascii_whitespace()).map(|i| i + 1).unwrap_or(s);
    let mut u = String::with_capacity(c.len()); u.push_str(&c[..rs]); if rs < s { u.push('\n'); } u.push_str(&c[e..]);
    if !keep { u = replace_head_symbol(&u, sym, "ignore", dialect_of(p), RefsMode::HeadOnly); }
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "remove".into(), detail: d }, o => o, })?; Ok(u)
}

// ── move ───────────────────────────────────────────────────────

pub fn move_form(reg: &Registry, path: &str, sym: &str, after: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let removed = remove_form(reg, path, sym, true)?; let p2 = reg.plugin_for_path(path)?;
    let ins = insert_after(p2, &removed, after, ft.trim())?;
    p2.check_file(&ins).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "move".into(), detail: d }, o => o, })?; Ok(ins)
}

// ── substitute ─────────────────────────────────────────────────

pub fn substitute(reg: &Registry, path: &str, sym: &str, pat: &str, rep: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let (s, e) = find_sexp(ft, pat).ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
    let nf = format!("{}{}{}", &ft[..s], rep, &ft[e..]); let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "substitute".into(), detail: d }, o => o, })?; Ok(u)
}

// ── extract ────────────────────────────────────────────────────

pub fn extract(reg: &Registry, path: &str, sym: &str, pat: &str, name: &str, params: &[&str]) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let (s, e) = find_sexp(ft, pat).ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
    let ex = &ft[s..e]; let fv = if params.is_empty() { detect_syms(ex) } else { params.to_vec() };
    let ps = if fv.is_empty() { "()".to_string() } else { format!("({})", fv.join(" ")) };
    let nd = if p.id() == "scheme" { format!("(define ({name} {ps})\n  {ex})\n") } else { format!("(defun {name} {ps}\n  {ex})\n") };
    let call = if fv.is_empty() { format!("({name})") } else { format!("({} {})", name, fv.join(" ")) };
    let uf = format!("{}{}{}", &ft[..s], &call, &ft[e..]); let as_ = replace_node(p, &c, sym, &uf)?;
    let p2 = reg.plugin_for_path(path)?; let ins = insert_after(p2, &as_, sym, &nd)?;
    p2.check_file(&ins).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "extract".into(), detail: d }, o => o, })?; Ok(ins)
}

fn detect_syms(sexp: &str) -> Vec<&str> {
    let mut seen = std::collections::BTreeSet::new(); let b = sexp.as_bytes(); let mut i = 0;
    while i < b.len() { i = skip_sp(b, i); if i >= b.len() { break; }
        match b[i] { b'(' | b')' => { i += 1; }
            b'"' => { i += 1; while i < b.len() && b[i] != b'"' { if b[i] == b'\\' { i += 2; continue; } i += 1; } i += 1; }
            b'\'' | b'`' | b',' => { i += 1; }
            b';' => { while i < b.len() && b[i] != b'\n' { i += 1; } }
            _ => { let s = i; i = skip_sym(b, i); let sym = &sexp[s..i];
                if !sym.is_empty() && !sym.starts_with(|c: char| c.is_ascii_digit())
                    && !matches!(sym, "nil"|"t"|"if"|"let"|"progn"|"lambda"|"defun"|"define"|"setq"|"when"|"unless"|"cond"|"and"|"or"|"not"|"list"|"car"|"cdr"|"cons"|"+"|"-"|"*"|"/"|">"|"<"|"=") { seen.insert(sym); }
            }
        }
    }
    seen.into_iter().collect()
}

// ── wrap ────────────────────────────────────────────────────────

pub fn wrap_body(reg: &Registry, path: &str, sym: &str, wrapper: &str, args: &[(&str, &str)]) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let b = body_range(ft)?; let nf = format!("{}{}{}", &ft[..b.0], make_wrapper(wrapper, args, &ft[b.0..b.1])?, &ft[b.1..]);
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
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let nf = if let Some(tf) = with { let b = body_range(ft)?; format!("{}{}{}", &ft[..b.0], instr_body(&ft[b.0..b.1], tf), &ft[b.1..]) }
        else if let (Some(pat), Some(wrp)) = (at, wrap) { let (s, e) = find_sexp(ft, pat).ok_or_else(|| Error::Message(format!("pattern not found: `{pat}`")))?;
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
/// Returns `None` if `ft` is not a flattenable function definition.
fn def_params_and_body(ft: &str, d: Dialect) -> Option<(Vec<String>, String)> {
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

/// Inline every `(sym arg…)` call site by substituting the call's arguments
/// into `body`. Errors on argument-count mismatch.
fn inline_calls(content: &str, sym: &str, params: &[String], body: &str, d: Dialect) -> Result<String, Error> {
    let b = content.as_bytes();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => { let e = lisp_sitter_core::sexp_reader::skip_string(b, i).unwrap_or(b.len()); out.push_str(&content[i..e]); i = e; }
            b';' => { let e = lisp_sitter_core::sexp_reader::skip_line_comment(b, i).unwrap_or(b.len()); out.push_str(&content[i..e]); i = e; }
            b'#' if i + 1 < b.len() && b[i + 1] == b'|' => { let e = lisp_sitter_core::sexp_reader::skip_block_comment(b, i).unwrap_or(b.len()); out.push_str(&content[i..e]); i = e; }
            b'#' if i + 1 < b.len() && b[i + 1] == b'\\' => { let e = skip_sexp_d(b, i, d); out.push_str(&content[i..e]); i = e; }
            b'?' if d == Dialect::Elisp => { let e = skip_sexp_d(b, i, d); out.push_str(&content[i..e]); i = e; }
            b'(' => {
                let ws = skip_sp(b, i + 1);
                let sym_end = skip_sym(b, ws);
                if sym_end > ws && &content[ws..sym_end] == sym {
                    let call_end = skip_sexp_d(b, i, d);
                    let call_text = &content[i..call_end];
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
                    out.push_str(&inlined);
                    i = call_end;
                } else {
                    out.push('(');
                    i += 1;
                }
            }
            _ => { let l = utf8_len(b[i]); let e = (i + l).min(b.len()); out.push_str(&content[i..e]); i = e; }
        }
    }
    Ok(out)
}

pub fn flatten(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = ops_read(path)?;
    let p = reg.plugin_for_path(path)?;
    let d = dialect_of(p);
    let ft = get_form_text(p, &c, sym)?.to_string();

    let (params, body) = def_params_and_body(&ft, d).ok_or_else(|| {
        Error::Message(format!("flatten: `{sym}` is not a flattenable function definition"))
    })?;
    if params.iter().any(|pn| pn.starts_with('&') || pn.starts_with('(') || pn.is_empty()) {
        return Err(Error::Message(
            "flatten supports only simple positional parameters (no &rest/&optional/&key/destructuring)".into(),
        ));
    }
    if replace_head_symbol(&body, sym, "\u{0}", d, RefsMode::HeadOnly) != body {
        return Err(Error::Message(format!("flatten: `{sym}` is recursive; cannot inline")));
    }

    // Inline call sites first, then remove the (now-unreferenced) definition.
    let inlined = inline_calls(&c, sym, &params, &body, d)?;
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

// ── convert-let ────────────────────────────────────────────────

pub fn convert_let(reg: &Registry, path: &str, sym: &str, target: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let nf = ft.replacen(if target == "let*" { "(let " } else { "(let* " },
                       if target == "let*" { "(let* " } else { "(let " }, 1);
    let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "convert-let".into(), detail: d }, o => o, })?; Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_registry;

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
        assert_eq!(find_sexp("(+ x 1)", "(+ x 1)"), Some((0, 7)));
        assert_eq!(find_sexp("calls (foo) and (bar)", "(foo)"), Some((6, 11)));
    }

    #[test]
    fn test_find_sexp_not_found() {
        assert_eq!(find_sexp("(defun foo ())", "(bar)"), None);
    }

    #[test]
    fn test_body_range() {
        let ft = "(defun foo (x)\n  (+ x 1))";
        let range = body_range(ft).unwrap();
        assert_eq!(&ft[range.0..range.1], "(+ x 1)");
    }

    #[test]
    fn test_body_range_trivial() {
        // () has an empty body between the parens — not an error
        assert!(body_range("()").is_ok());
    }

    #[test]
    fn test_replace_name_in_form() {
        let result = replace_name_in_form("(defun foo (x) (+ x 1))", "foo", "bar");
        assert_eq!(result, "(defun bar (x) (+ x 1))");
    }

    #[test]
    fn test_replace_name_in_form_no_opener() {
        assert_eq!(replace_name_in_form("just a string", "x", "y"), "just a string");
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
        let syms = detect_syms("(+ x 1)");
        // built-in operators are skipped
        assert!(!syms.contains(&"+"));
        assert!(!syms.contains(&"1"));
        // x is a variable — could be detected (depends on skip list)
        // just check no panics and shape is right
        assert!(syms.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn test_replace_head_symbol() {
        let c = "(foo 1)\n(bar (foo 2))\n(ignore)";
        let result = replace_head_symbol(c, "foo", "baz", Dialect::Generic, RefsMode::HeadOnly);
        assert_eq!(result, "(baz 1)\n(bar (baz 2))\n(ignore)");
    }

    #[test]
    fn test_replace_head_symbol_skips_strings_and_comments() {
        let c = "(foo)\n(bar (message \"call (foo) here\")) ; foo in comment\n";
        let result = replace_head_symbol(c, "foo", "baz", Dialect::Generic, RefsMode::HeadOnly);
        assert!(result.contains("(baz)"));
        assert!(result.contains("\"call (foo) here\""), "string must be untouched: {result}");
        assert!(result.contains("; foo in comment"), "comment must be untouched: {result}");
    }

    #[test]
    fn test_replace_head_symbol_sharp_refs() {
        // HeadAndSharp: renames #'foo + (foo), but not plain 'foo
        let c = "(foo)\n(add-hook 'h #'foo)\n(setq x 'foo)";
        let got = replace_head_symbol(c, "foo", "baz", Dialect::Generic, RefsMode::HeadAndSharp);
        assert!(got.contains("(baz)"), "{got}");
        assert!(got.contains("#'baz"), "#' should be renamed: {got}");
        assert!(got.contains("'foo"), "plain 'foo should not be renamed without --refs: {got}");
    }

    #[test]
    fn test_replace_head_symbol_all_refs() {
        // AllRefs: renames #'foo, 'foo, and (foo)
        let c = "(foo)\n(add-hook 'h #'foo)\n(setq sym 'foo)";
        let got = replace_head_symbol(c, "foo", "baz", Dialect::Generic, RefsMode::AllRefs);
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
        let syms = detect_syms(r#"(some-fn "string with (parens)" ; comment
  (+ 1 2))"#);
        assert!(!syms.contains(&"+"));
        assert!(syms.contains(&"some-fn"));
    }

    #[test]
    fn test_replace_name_in_form_inner_paren() {
        // Form like (defmethod foo ((x integer) ...)) — inner parens before name
        let result = replace_name_in_form("(defmethod foo ((x integer) body)", "foo", "bar");
        assert_eq!(result, "(defmethod bar ((x integer) body)");
    }

    #[test]
    fn test_replace_name_in_form_no_name_match() {
        let result = replace_name_in_form("(foo bar baz)", "qux", "quux");
        assert_eq!(result, "(foo bar baz)");
    }

    #[test]
    fn test_replace_name_in_form_inner_paren_with_match() {
        // ah starts with '(' and inner name matches (defmethod-like)
        let result = replace_name_in_form("(defmethod foo ((x integer)) body)", "foo", "bar");
        assert_eq!(result, "(defmethod bar ((x integer)) body)");
    }

    #[test]
    fn test_detect_syms_with_quote() {
        // tick mark (') should be skipped
        let syms = detect_syms("'(1 2 3)");
        // no real variable bindings, should be empty or just contain quoted content
        assert!(syms.iter().all(|s| !s.is_empty()));
    }
}
