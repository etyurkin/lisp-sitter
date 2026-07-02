#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FormInfo {
    pub name: Option<String>,
    pub label: String,
    pub start: usize,
    pub end: usize,
}

/// Which kind of syntactic reference a [`SymbolRef`] represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// `(sym …)` — symbol is the head of a function-call list.
    CallHead,
    /// `#'sym` — sharp-quoted function reference.
    SharpQuote,
    /// `'sym` — plain quoted symbol reference.
    Quote,
}

/// A single occurrence of a symbol used as a syntactic reference.
/// Strings, comments, char literals, and let-binding variable positions are
/// excluded before this struct is produced.
#[derive(Debug, Clone)]
pub struct SymbolRef {
    /// Byte of `(` (CallHead), `#` (SharpQuote), or `'` (Quote).
    pub form_start: usize,
    /// Byte of the first character of the symbol token.
    pub sym_start: usize,
    /// Byte past the last character of the symbol token.
    pub sym_end: usize,
    pub kind: RefKind,
}

pub trait LanguagePlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn extensions(&self) -> &[&'static str];
    fn matches_path(&self, path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        self.extensions().iter().any(|ext| path.ends_with(ext))
    }
    fn top_level_forms(&self, content: &str) -> crate::Result<Vec<FormInfo>>;
    fn check_file(&self, content: &str) -> crate::Result<()>;
    fn check_node(&self, node: &str) -> crate::Result<()>;
    fn outline(&self, content: &str) -> crate::Result<String>;
    fn list_forms(&self, content: &str) -> crate::Result<Vec<FormInfo>>;
    fn tree_depth(&self, content: &str, depth: usize) -> crate::Result<String> {
        if depth <= 1 {
            self.outline(content)
        } else {
            Err(crate::Error::NotImplemented("tree_depth".into()))
        }
    }
    fn node_bounds(&self, content: &str, symbol: &str) -> crate::Result<(usize, usize)>;
    fn semantic_check(&self, content: &str) -> Vec<String> {
        let _ = content;
        Vec::new()
    }

    /// Whether `name` is a known special form, macro, or built-in function of
    /// this dialect — i.e. a symbol that resolves globally without a project
    /// definition. Used by project analysis to suppress unresolved-call
    /// warnings. The default returns `false`; each plugin overrides it with a
    /// curated (non-exhaustive) set.
    fn is_known_global(&self, name: &str) -> bool {
        let _ = name;
        false
    }

    /// Return descriptions of MISSING tokens and ERROR nodes found by the
    /// tree-sitter parser.  Empty when the file is structurally clean or when
    /// tree-sitter is unavailable for this language.
    fn find_errors(&self, content: &str) -> Vec<String> {
        let _ = content;
        Vec::new()
    }

    /// Return `(body_start, body_end)` within `form_text` — the byte range of
    /// the actual executable body after the head, name, qualifiers, parameter
    /// list, and any docstring / `declare` preamble.
    fn form_body_range(&self, form_text: &str) -> Option<(usize, usize)> {
        let _ = form_text;
        None
    }

    /// Extract parameter names and body text from a function definition form.
    /// Returns `None` when the form is not a flattenable function definition.
    fn form_params_and_body(&self, form_text: &str) -> Option<(Vec<String>, String)> {
        let _ = form_text;
        None
    }

    /// Rename the function-name token inside `form_text` from `old` to `new`.
    /// Returns the modified form text, or `None` when tree-sitter is unavailable.
    fn form_rename_name(&self, form_text: &str, old: &str, new: &str) -> Option<String> {
        let _ = (form_text, old, new);
        None
    }

    /// Find the first occurrence of `pattern` as a complete syntactic
    /// sub-expression within `content`, excluding strings and comments.
    /// Returns `Some(Some(range))` when found, `Some(None)` when the tree-sitter
    /// search completed but found nothing, or `None` when tree-sitter is
    /// unavailable for this content — in which case the caller should fall back
    /// to a character-level scanner.
    fn find_sexp_in(&self, _content: &str, _pattern: &str) -> Option<Option<(usize, usize)>> {
        None // default: no tree-sitter impl, caller uses character scanner
    }

    /// Find every syntactic reference to `symbol` in `content`:
    /// call-head positions `(symbol …)`, sharp-quotes `#'symbol`, and
    /// plain quotes `'symbol`. Strings, comments, char literals, and
    /// let-binding variable positions are excluded automatically.
    ///
    /// The default implementation uses the character-level scanner from
    /// [`crate::edit::find_callers_in`] and does not classify quotes.
    /// Each language plugin overrides this with a tree-sitter AST walk.
    fn find_symbol_refs(&self, content: &str, symbol: &str) -> Vec<SymbolRef> {
        let dialect = if self.id() == "elisp" {
            crate::sexp_reader::Dialect::Elisp
        } else {
            crate::sexp_reader::Dialect::Generic
        };
        crate::edit::find_callers_in(content, symbol, dialect)
            .into_iter()
            .map(|form_start| {
                // sym_start: skip `(` and any whitespace
                let b = content.as_bytes();
                let mut s = form_start + 1;
                while s < b.len() && matches!(b[s], b' ' | b'\t' | b'\n' | b'\r') {
                    s += 1;
                }
                let mut e = s;
                while e < b.len() && !matches!(b[e], b'(' | b')' | b' ' | b'\t' | b'\n' | b'\r') {
                    e += 1;
                }
                SymbolRef {
                    form_start,
                    sym_start: s,
                    sym_end: e,
                    kind: RefKind::CallHead,
                }
            })
            .collect()
    }
}
