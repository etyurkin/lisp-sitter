use lisp_sitter_core::edit::{get_form_text, insert_after, replace_node};
use lisp_sitter_core::{Error, Registry};

// ── helpers ────────────────────────────────────────────────────

fn ops_read(path: &str) -> Result<String, Error> {
    let p = std::path::Path::new(path);
    if p.exists() { std::fs::read_to_string(p).map_err(|e| Error::Message(format!("read {path}: {e}"))) }
    else { Err(Error::Message(format!("file not found: {path}"))) }
}

fn skip_sp(bytes: &[u8], mut i: usize) -> usize { while i < bytes.len() && (bytes[i] as char).is_whitespace() { i += 1; } i }
fn skip_sym(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && !(bytes[i] as char).is_whitespace() && bytes[i] != b'(' && bytes[i] != b')' { i += 1; } i
}
fn skip_sexp(bytes: &[u8], mut i: usize) -> usize {
    if i >= bytes.len() { return i; }
    if bytes[i] == b'(' { let mut d = 1u32; i += 1;
        while i < bytes.len() && d > 0 { match bytes[i] { b'(' => d += 1, b')' => d -= 1, b'"' => { i += 1; while i < bytes.len() && bytes[i] != b'"' { if bytes[i] == b'\\' { i += 2; continue; } i += 1; } } _ => {} } i += 1; }
    } else { i = skip_sym(bytes, i); }
    i
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

fn replace_call_sites(c: &str, old: &str, new: &str) -> String {
    c.replace(&format!("({} ", old), &format!("({} ", new)).replace(&format!("({})", old), &format!("({})", new))
}

// ── rename ─────────────────────────────────────────────────────

pub fn rename(reg: &Registry, path: &str, old: &str, new: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?;
    let u = replace_node_header(p, &c, old, new)?;
    let u = replace_call_sites(&u, old, new);
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
    if ah.starts_with('(') { let inner = &ah[1..]; let ne = inner.find(|c: char| c.is_whitespace() || c == ')').unwrap_or(inner.len());
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
    if !keep { u = replace_call_sites(&u, sym, "ignore"); }
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
    let p2 = reg.plugin_for_path(path)?; let ins = insert_after(p2, &as_, "__start__", &nd)?;
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
        let n = skip_sexp(b, i); let f = (&body[i..n]).trim();
        if !f.is_empty() { if !first { out.push('\n'); } out.push_str(&format!("(progn\n  {}\n  {})", trace, f)); first = false; }
        i = n; }
    out
}

// ── flatten ────────────────────────────────────────────────────

pub fn flatten(reg: &Registry, path: &str, sym: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let b = body_range(ft)?; let body = &ft[b.0..b.1];
    let mut u = c.clone(); let pat = format!("({}", sym); let def_start = c.find(ft).unwrap_or(0);
    loop { let pos = u[..].find(&pat); let a = match pos { Some(p) => p, None => break };
        if a >= def_start && a < def_start + ft.len() { let skip_to = a + 1; let _ = skip_to; /* skip def */ let next = if a + 1 < u.len() { a + 1 } else { break }; u = u[..a].to_string() + &u[next..]; continue; }
        let se = skip_sexp(u.as_bytes(), a); let mut n = String::with_capacity(u.len());
        n.push_str(&u[..a]); n.push_str(body.trim()); n.push_str(&u[se..]); u = n; }
    let u = remove_form(reg, path, sym, true)?;
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "flatten".into(), detail: d }, o => o, })?; Ok(u)
}

// ── convert-let ────────────────────────────────────────────────

pub fn convert_let(reg: &Registry, path: &str, sym: &str, target: &str) -> Result<String, Error> {
    let c = ops_read(path)?; let p = reg.plugin_for_path(path)?; let ft = get_form_text(p, &c, sym)?;
    let nf = ft.replacen(if target == "let*" { "(let " } else { "(let* " },
                       if target == "let*" { "(let* " } else { "(let " }, 1);
    let u = replace_node(p, &c, sym, &nf)?;
    p.check_file(&u).map_err(|e| match e { Error::Syntax(d) => Error::SyntaxAfterEdit { operation: "convert-let".into(), detail: d }, o => o, })?; Ok(u)
}
