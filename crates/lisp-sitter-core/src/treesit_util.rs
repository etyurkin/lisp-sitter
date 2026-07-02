use tree_sitter::Node;

use crate::definers::DefinerSet;
use crate::plugin::{RefKind, SymbolRef};
use crate::sexp_reader::Dialect;
use crate::FormInfo;

pub fn node_text(node: Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
}

/// One [`FormInfo`] per recognized top-level definition under `root`.
///
/// Tree-sitter supplies only the byte boundaries of each top-level child; the
/// head keyword and name are read from the child's source text via `set`, so
/// this is identical whether the grammar emits a dedicated def node or a plain
/// list. Comments and non-definer forms are skipped.
pub fn forms_from_tree(content: &str, root: Node, set: &DefinerSet) -> Vec<FormInfo> {
    let mut forms = Vec::new();
    for child in child_nodes(root) {
        if child.kind() == "comment" {
            continue;
        }
        let (start, end) = (child.start_byte(), child.end_byte());
        if let Some((head, name)) = set.classify(&content[start..end]) {
            forms.push(FormInfo {
                name: Some(name.clone()),
                label: format!("{head}:{name}"),
                start,
                end,
            });
        }
    }
    forms
}

/// The s-expression fallback: recognized definitions found by byte scanning,
/// using the same [`DefinerSet`] as the tree path.
pub fn fallback_forms(content: &str, set: &DefinerSet, dialect: Dialect) -> Vec<FormInfo> {
    crate::sexp_scan::top_level_definer_forms(content, set, dialect)
        .unwrap_or_default()
        .into_iter()
        .map(|f| FormInfo {
            name: Some(f.name.clone()),
            label: format!("{}:{}", f.head, f.name),
            start: f.start,
            end: f.end,
        })
        .collect()
}

/// Byte range of the first form whose name matches `target`.
pub fn bounds_in_forms(forms: &[FormInfo], target: &str) -> Option<(usize, usize)> {
    forms
        .iter()
        .find(|f| f.name.as_deref() == Some(target))
        .map(|f| (f.start, f.end))
}

pub fn child_nodes(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

pub fn meaningful_children(node: Node) -> Vec<Node> {
    child_nodes(node)
        .into_iter()
        .filter(|c: &Node| !matches!(c.kind(), "(" | ")"))
        .collect()
}

pub fn validate_treesit(content: &str, has_parse_errors: bool) -> crate::Result<()> {
    if let Some(err) = crate::scan::scan_parens(content) {
        return Err(crate::error::Error::Syntax(err));
    }
    if has_parse_errors {
        return Err(crate::error::Error::Syntax(crate::position::error_at(
            content,
            0,
            "tree-sitter parse error",
        )));
    }
    Ok(())
}

/// Recursively walk a tree‑sitter node up to `max_depth` and produce an
/// indented outline.  Only nodes that look like "forms" (lists, function
/// definitions, etc.) are printed — leaf tokens are skipped.
pub fn recursive_outline(content: &str, node: tree_sitter::Node, max_depth: usize) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        write_sub_tree(content, child, max_depth, 0, &mut out);
    }
    out
}

fn write_sub_tree(
    content: &str,
    node: tree_sitter::Node,
    max_depth: usize,
    cur_depth: usize,
    out: &mut String,
) {
    if !is_form_like(&node) {
        return;
    }

    // Emit label for this node
    if let Some(label) = form_label(content, node) {
        if !out.is_empty() {
            out.push('\n');
        }
        for _ in 0..cur_depth {
            out.push_str("  ");
        }
        let (line, col) = crate::position::line_column(content, node.start_byte());
        out.push_str(&format!("{label}@{line}:{col}"));
    }

    if cur_depth >= max_depth {
        return;
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        write_sub_tree(content, child, max_depth, cur_depth + 1, out);
    }
}

/// True if a tree‑sitter node represents a "form" we'd show in a tree view:
/// lists, function/macro/class definitions, special forms, vectors.
fn is_form_like(node: &tree_sitter::Node) -> bool {
    matches!(
        node.kind(),
        "list"
            | "list_lit"
            | "function_definition"
            | "macro_definition"
            | "special_form"
            | "defun"
            | "defmacro"
            | "defclass"
            | "defgeneric"
            | "defmethod"
            | "define"
            | "define_syntax"
            | "define_library"
            | "let"
            | "let_binding"
            | "body"
            | "function_body"
            | "parenthesized_list"
            | "vector"
            | "quoted"
    )
}

/// Extract a human‑readable label for a form node. Prefers the form's head
/// keyword text (e.g. `defun`) over the grammar node kind so dedicated nodes
/// and plain lists label consistently.
fn form_label(content: &str, node: tree_sitter::Node) -> Option<String> {
    let kids = meaningful_children(node);

    // Head = first meaningful child's text (the keyword: defun, require, …).
    let head = kids
        .first()
        .map(|n| node_text(*n, content).trim().to_string())
        .filter(|s| !s.is_empty());

    // Name = the `name` field if present, else a second symbol child.
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, content).trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            kids.get(1)
                .filter(|n| matches!(n.kind(), "sym_lit" | "symbol"))
                .map(|n| node_text(*n, content).trim().to_string())
                .filter(|s| !s.is_empty())
        });

    match (head, name) {
        (Some(h), Some(n)) => Some(format!("{h}:{n}")),
        (Some(h), None) => Some(h),
        (None, _) => Some(node.kind().to_string()),
    }
}

/// Check if a form has a docstring — a string literal right after the arglist
/// (for defun/define/defclass) or as the third argument (for defvar/defcustom).
///
/// The scanner watches for:
/// - A `)` at depth 2 → that closes the arglist → `found_after_args = true`.
/// - A `)` at depth 1 on a non‑empty inner list → also marks it.
/// - Then the next `"` signals the docstring.
pub fn has_docstring(form_text: &str) -> bool {
    let mut depth = 0i32;
    let mut found_after_args = false;
    let bytes = form_text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                // depth==2: closing the arglist (defun f (x) "…")
                // The outer `)` of the defun is at depth==1 and is ignored.
                if depth == 2 {
                    found_after_args = true;
                }
                depth -= 1;
                i += 1;
            }
            b'"' if found_after_args => {
                // Found a string literal right after the arglist — that's the docstring.
                return true;
            }
            b';' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'#' if i + 1 < bytes.len() && bytes[i + 1] == b'|' => {
                while i + 1 < bytes.len() && !(bytes[i] == b'|' && bytes[i + 1] == b'#') {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 2;
                }
            }
            _ => {
                i += 1;
            }
        }
        if depth < 0 {
            break;
        }
    }
    false
}

pub fn pos_label(content: &str, start: usize, label: &str) -> String {
    crate::position::pos_label(content, start, label)
}

/// Walk a tree-sitter AST and collect human-readable descriptions of every
/// MISSING token and ERROR node, with their line/column positions.
pub fn find_error_nodes(content: &str, root: Node) -> Vec<String> {
    let mut out = Vec::new();
    collect_errors(content, root, &mut out);
    out
}

fn collect_errors(content: &str, node: Node, out: &mut Vec<String>) {
    if node.is_missing() {
        let (line, col) = crate::position::line_column(content, node.start_byte());
        out.push(format!("line {line}, col {col}: missing '{}'", node.kind()));
        return;
    }
    if node.kind() == "ERROR" {
        let (line, col) = crate::position::line_column(content, node.start_byte());
        out.push(format!("line {line}, col {col}: parse error"));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_errors(content, child, out);
    }
}

// ── Sub-expression finder ──────────────────────────────────────────────────

/// Find the first occurrence of `pattern` as a complete syntactic node within
/// the tree rooted at `root`. Strings and comments are skipped automatically.
///
/// Returns `Some((start, end))` where `content[start..end] == pattern`, or
/// `None` if not found.
pub fn find_sexp_in_tree(content: &str, pattern: &str, root: Node) -> Option<(usize, usize)> {
    find_sexp_in_node(content, pattern, root)
}

fn find_sexp_in_node(content: &str, pat: &str, node: Node) -> Option<(usize, usize)> {
    let kind = node.kind();
    if kind.contains("string") || kind.contains("comment") {
        return None;
    }

    let s = node.start_byte();
    let e = node.end_byte();

    // Prune: subtree nodes are never larger than the current node.
    if e.saturating_sub(s) < pat.len() {
        return None;
    }

    if &content[s..e] == pat {
        return Some((s, e));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(r) = find_sexp_in_node(content, pat, child) {
            return Some(r);
        }
    }
    None
}

// ── Definition-form structural analysis ───────────────────────────────────

/// Results of analysing a single top-level definition form (defun, defmacro,
/// define, defmethod, …).  All byte offsets are into the form text that was
/// passed to `analyze_def_form`.
pub struct DefFormInfo {
    /// Byte range `[body_start, body_end)` of the actual body — after the
    /// head keyword, name, qualifiers, parameter list, and any preamble
    /// (docstrings / `declare` forms).
    pub body_start: usize,
    pub body_end: usize,
    /// Parameter names taken from the parameter list.  For CL specialised
    /// params `((x MyClass) y)` the name part (`x`) is extracted; `&rest`
    /// and friends are kept verbatim so the caller can reject them.
    pub param_names: Vec<String>,
    /// Byte range `[name_start, name_end)` of the function-name token inside
    /// the form text.  For curried Scheme `(define (name params) …)` this
    /// points to the `name` inside the signature list.
    pub name_start: usize,
    pub name_end: usize,
}

/// Analyse a parsed definition form and return its structural parts.
/// `root` should be the `source_file` node returned by the tree-sitter parser
/// when `form_text` is passed as input.  Returns `None` if the form doesn't
/// look like a definition (fewer than 3 meaningful children, etc.).
pub fn analyze_def_form(form_text: &str, root: Node) -> Option<DefFormInfo> {
    let b = form_text.as_bytes();

    // The actual list node is the first meaningful child of source_file.
    let form = child_nodes(root)
        .into_iter()
        .find(|n| !n.is_extra() && !matches!(n.kind(), "comment"))?;

    // Significant children of the list (no parens, no comments).
    let ch: Vec<Node> = child_nodes(form)
        .into_iter()
        .filter(|n| !matches!(n.kind(), "(" | ")") && !n.is_extra())
        .collect();

    if ch.len() < 2 {
        return None;
    }

    // ── Detect curried Scheme define: (define (name params…) body…) ──────
    let name1_s = ch[1].start_byte();
    if b.get(name1_s) == Some(&b'(') {
        // ch[1] = (name params…) signature
        let sig_ch: Vec<Node> = child_nodes(ch[1])
            .into_iter()
            .filter(|n| !matches!(n.kind(), "(" | ")") && !n.is_extra())
            .collect();
        if sig_ch.is_empty() {
            return None;
        }

        let actual_name = sig_ch[0];
        let param_names: Vec<String> = sig_ch[1..]
            .iter()
            .map(|n| form_text[n.start_byte()..n.end_byte()].to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let after = skip_preamble_ch(&ch[2..], form_text, b);
        if after.is_empty() {
            return None;
        }

        return Some(DefFormInfo {
            body_start: after[0].start_byte(),
            body_end: after.last().unwrap().end_byte(),
            param_names,
            name_start: actual_name.start_byte(),
            name_end: actual_name.end_byte(),
        });
    }

    // ── Normal form: ch[1] is the name symbol ────────────────────────────
    let name_start = ch[1].start_byte();
    let name_end = ch[1].end_byte();

    // Skip qualifier tokens/lists that precede the parameter list.
    // A qualifier is a keyword (starts with ':') or a list whose first
    // meaningful child starts with ':'.
    let mut i = 2usize;
    while i < ch.len() {
        let cs = ch[i].start_byte();
        if b.get(cs) == Some(&b':') {
            i += 1;
            continue;
        } // bare keyword
        if b.get(cs) == Some(&b'(') {
            let first_ch = child_nodes(ch[i])
                .into_iter()
                .find(|n| !matches!(n.kind(), "(" | ")") && !n.is_extra());
            if first_ch
                .map(|n| b.get(n.start_byte()) == Some(&b':'))
                .unwrap_or(false)
            {
                i += 1;
                continue; // qualifier list like (:before :after)
            }
            break; // actual param list
        }
        break;
    }
    if i >= ch.len() {
        return None;
    }

    let param_list = ch[i];
    let param_names: Vec<String> = child_nodes(param_list)
        .into_iter()
        .filter(|n| !matches!(n.kind(), "(" | ")") && !n.is_extra())
        .map(|n| {
            let t = &form_text[n.start_byte()..n.end_byte()];
            if t.starts_with('(') {
                // Specialised CL param (var type) — extract var name.
                child_nodes(n)
                    .into_iter()
                    .find(|c| !matches!(c.kind(), "(" | ")") && !c.is_extra())
                    .map(|c| form_text[c.start_byte()..c.end_byte()].to_string())
                    .unwrap_or_default()
            } else {
                t.to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    let after = skip_preamble_ch(&ch[i + 1..], form_text, b);
    if after.is_empty() {
        return None;
    }

    Some(DefFormInfo {
        body_start: after[0].start_byte(),
        body_end: after.last().unwrap().end_byte(),
        param_names,
        name_start,
        name_end,
    })
}

/// Skip leading docstrings (`"…"`) and `(declare …)` forms in a child slice.
fn skip_preamble_ch<'a>(nodes: &'a [Node<'a>], content: &str, b: &[u8]) -> &'a [Node<'a>] {
    let mut s = 0;
    while s < nodes.len() {
        let cs = nodes[s].start_byte();
        if b.get(cs) == Some(&b'"') {
            s += 1;
            continue;
        } // docstring
        if b.get(cs) == Some(&b'(') {
            let head = child_nodes(nodes[s])
                .into_iter()
                .find(|n| !matches!(n.kind(), "(" | ")") && !n.is_extra())
                .map(|n| &content[n.start_byte()..n.end_byte()]);
            if head == Some("declare") {
                s += 1;
                continue;
            }
        }
        break;
    }
    &nodes[s..]
}

// ── Symbol reference finder ────────────────────────────────────────────────

/// Keywords that introduce a binding list as their first argument.
/// Used to detect `(sym init)` binding specs that should not be treated as calls.
pub fn is_let_keyword(kw: &str) -> bool {
    matches!(
        kw,
        "let"
            | "let*"
            | "letrec"
            | "letrec*"
            | "cl-let*"
            | "pcase-let"
            | "pcase-let*"
            | "if-let"
            | "if-let*"
            | "when-let"
            | "when-let*"
            | "and-let*"
            | "fluid-let"
            | "flet"
            | "labels"
            | "macrolet"
            | "symbol-macrolet"
            | "let-values"
            | "let*-values"
            | "receive"
    )
}

/// Walk `root` and collect every syntactic reference to `symbol`, excluding
/// string/comment nodes and let-binding variable positions.
///
/// This function is language-agnostic: it identifies symbol tokens by node
/// kind (`"symbol"` or `"sym_lit"`) and classifies references by inspecting
/// the raw bytes at the parent node's start position rather than relying on
/// grammar-specific node-kind names.
pub fn find_symbol_refs_in_tree(content: &str, root: Node, symbol: &str) -> Vec<SymbolRef> {
    let b = content.as_bytes();
    let sym_bytes = symbol.as_bytes();
    let mut out = Vec::new();
    collect_refs(b, root, sym_bytes, &mut out);
    out
}

fn collect_refs(b: &[u8], node: Node, sym: &[u8], out: &mut Vec<SymbolRef>) {
    let kind = node.kind();

    // Skip strings and comments entirely — no refs inside them.
    if kind.contains("string") || kind.contains("comment") {
        return;
    }

    let s = node.start_byte();
    let e = node.end_byte();

    // Match symbol tokens.
    if matches!(kind, "symbol" | "sym_lit" | "symbol_lit") && &b[s..e] == sym {
        if let Some(r) = classify_ref(b, node) {
            out.push(r);
        }
        return; // symbols have no children
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_refs(b, child, sym, out);
    }
}

fn classify_ref(b: &[u8], sym_node: Node) -> Option<SymbolRef> {
    let sym_s = sym_node.start_byte();
    let sym_e = sym_node.end_byte();
    let parent = sym_node.parent()?;
    let ps = parent.start_byte();

    // Determine reference kind from the bytes at the parent node's start.
    if ps + 1 < b.len() && b[ps] == b'#' && b[ps + 1] == b'\'' {
        return Some(SymbolRef {
            form_start: ps,
            sym_start: sym_s,
            sym_end: sym_e,
            kind: RefKind::SharpQuote,
        });
    }
    if b[ps] == b'\'' {
        return Some(SymbolRef {
            form_start: ps,
            sym_start: sym_s,
            sym_end: sym_e,
            kind: RefKind::Quote,
        });
    }
    if b[ps] == b'(' {
        // Check that this symbol is the head (first meaningful child) of the list.
        if !is_head_of_list(sym_node, parent) {
            return None;
        }
        // Filter out let-binding variable positions.
        if is_binding_var_position(parent, b) {
            return None;
        }
        return Some(SymbolRef {
            form_start: ps,
            sym_start: sym_s,
            sym_end: sym_e,
            kind: RefKind::CallHead,
        });
    }
    None
}

fn is_head_of_list(sym: Node, parent: Node) -> bool {
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        let k = child.kind();
        if matches!(k, "(" | ")") || child.is_extra() {
            continue;
        }
        return child.start_byte() == sym.start_byte();
    }
    false
}

/// Return true if `call_head_list` (the `(sym …)` form) is in a let-binding
/// variable position — i.e. it is the first element of a binding spec `(sym init)`
/// whose parent is the binding list of a `let`/`let*`/etc. form.
fn is_binding_var_position(call_head_list: Node, b: &[u8]) -> bool {
    // call_head_list = (sym …) — could be a binding spec (sym init)
    let binding_list = match call_head_list.parent() {
        Some(n) => n,
        None => return false,
    };
    if b.get(binding_list.start_byte()) != Some(&b'(') {
        return false;
    }

    let let_form = match binding_list.parent() {
        Some(n) => n,
        None => return false,
    };
    if b.get(let_form.start_byte()) != Some(&b'(') {
        return false;
    }

    // binding_list must be the second sexp child (index 1) of let_form.
    let mut cursor = let_form.walk();
    let meaningful: Vec<_> = let_form
        .children(&mut cursor)
        .filter(|c| !matches!(c.kind(), "(" | ")") && !c.is_extra())
        .collect();
    if meaningful.len() < 2 {
        return false;
    }
    if meaningful[1].start_byte() != binding_list.start_byte() {
        return false;
    }

    // The first sexp child of let_form must be a let keyword.
    let head_start = meaningful[0].start_byte();
    let head_end = meaningful[0].end_byte();
    let head = std::str::from_utf8(&b[head_start..head_end]).unwrap_or("");
    is_let_keyword(head)
}

pub fn outline_lines(content: &str, forms: &[crate::FormInfo]) -> crate::Result<String> {
    if forms.is_empty() {
        if content.trim().is_empty() {
            return Ok("No forms".to_string());
        }
        return Ok("No forms".to_string());
    }
    Ok(forms
        .iter()
        .map(|f| crate::position::pos_label(content, f.start, &f.label))
        .collect::<Vec<_>>()
        .join("\n"))
}
