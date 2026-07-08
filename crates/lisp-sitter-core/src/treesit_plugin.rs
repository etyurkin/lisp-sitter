//! Generic tree-sitter-backed [`LanguagePlugin`] implementation.
//!
//! Everything structural — parsing, outlines, bounds, form/ref analysis,
//! validation — is identical across the Lisp dialects and lives here once. A
//! [`DialectSpec`] supplies only what genuinely differs between languages: the
//! grammar, the definer keywords, and a handful of dialect-specific renderings
//! and checks. Each language crate is then just a `DialectSpec` plus its
//! globals table.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use tree_sitter::{Language, Parser, Tree};

use crate::definers::Definer;
use crate::plugin::{FormInfo, LanguagePlugin, SymbolRef};
use crate::sexp_reader::Dialect;
use crate::treesit_util as tu;
use crate::{DefinerSet, Error, Result};

/// The language-specific half of a tree-sitter plugin. Structural behavior is
/// provided generically by [`TreesitPlugin`]; implementors supply only the
/// grammar and the parts that actually vary between dialects.
pub trait DialectSpec: Send + Sync {
    fn id(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    fn dialect(&self) -> Dialect {
        Dialect::Generic
    }
    /// The tree-sitter root node kind for a whole file (`source_file`,
    /// `program`, `source`, …).
    fn root_kind(&self) -> &'static str;
    /// The tree-sitter grammar for this dialect.
    fn language(&self) -> Language;
    /// Top-level definer keywords, before user-configured extras.
    fn base_definers(&self) -> Vec<Definer>;

    /// Wrap a bare node before validation in `check_node`. The default appends a
    /// newline so a trailing line comment doesn't swallow structure.
    fn wrap_node(&self, node: &str) -> String {
        format!("{}\n", node.trim())
    }

    /// Head symbol that neutralizes a removed definition's call sites when the
    /// caller asked to keep them. `values` is valid in CL and Scheme.
    fn noop_stub(&self) -> &'static str {
        "values"
    }

    /// Render a new function definition (used by `extract`). `param_list` is
    /// already parenthesized (`()` or `(a b)`).
    fn definition_template(&self, name: &str, param_list: &str, body: &str) -> String {
        format!("(defun {name} {param_list}\n  {body})\n")
    }

    /// Whether `name` resolves globally in this dialect (special form, macro, or
    /// built-in) — used to suppress unresolved-call warnings.
    fn is_known_global(&self, _name: &str) -> bool {
        false
    }

    /// Dialect-specific semantic warnings (missing docstrings, exports, …),
    /// given the already-computed top-level forms.
    fn semantic_check(&self, _content: &str, _forms: &[FormInfo]) -> Vec<String> {
        Vec::new()
    }
}

/// A [`LanguagePlugin`] built from a [`DialectSpec`]. Holds the resolved definer
/// set (base + user extras) and parses via a thread-local, per-language parser
/// cache so a `Parser` and its `set_language` are reused across calls.
pub struct TreesitPlugin {
    spec: Box<dyn DialectSpec>,
    definers: DefinerSet,
}

impl TreesitPlugin {
    pub fn new(spec: Box<dyn DialectSpec>) -> Self {
        let definers = DefinerSet::new(spec.base_definers());
        Self { spec, definers }
    }

    pub fn with_extra_definers(spec: Box<dyn DialectSpec>, extra: &[String]) -> Self {
        let mut definers = DefinerSet::new(spec.base_definers());
        definers.extend_keywords(extra);
        Self { spec, definers }
    }

    fn parse(&self, content: &str) -> Option<Tree> {
        parse_cached(self.spec.id(), &self.spec.language(), content)
    }

    fn has_parse_errors(&self, content: &str) -> bool {
        self.parse(content)
            .map(|t| t.root_node().has_error())
            .unwrap_or(true)
    }

    /// Dialect-aware structural validation: a balanced-paren scan honoring this
    /// dialect's char literals (so elisp `?\(` isn't miscounted), then a
    /// tree-sitter parse-error check.
    fn validate(&self, content: &str) -> Result<()> {
        if let Some(err) = crate::scan::scan_parens_in(content, self.spec.dialect()) {
            return Err(Error::Syntax(err));
        }
        if self.has_parse_errors(content) {
            return Err(Error::Syntax(crate::position::error_at(
                content,
                0,
                "tree-sitter parse error",
            )));
        }
        Ok(())
    }

    fn forms(&self, content: &str) -> Vec<FormInfo> {
        if let Some(tree) = self.parse(content) {
            let root = tree.root_node();
            if root.kind() == self.spec.root_kind() {
                let forms = tu::forms_from_tree(content, root, &self.definers);
                if !forms.is_empty() {
                    return forms;
                }
            }
        }
        tu::fallback_forms(content, &self.definers, self.spec.dialect())
    }
}

impl LanguagePlugin for TreesitPlugin {
    fn id(&self) -> &'static str {
        self.spec.id()
    }

    fn extensions(&self) -> &[&'static str] {
        self.spec.extensions()
    }

    fn dialect(&self) -> Dialect {
        self.spec.dialect()
    }

    fn noop_stub(&self) -> &'static str {
        self.spec.noop_stub()
    }

    fn definition_template(&self, name: &str, param_list: &str, body: &str) -> String {
        self.spec.definition_template(name, param_list, body)
    }

    fn top_level_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(self.forms(content))
    }

    fn list_forms(&self, content: &str) -> Result<Vec<FormInfo>> {
        Ok(self.forms(content))
    }

    fn check_file(&self, content: &str) -> Result<()> {
        self.validate(content)
    }

    fn check_node(&self, node: &str) -> Result<()> {
        let wrapped = self.spec.wrap_node(node);
        self.validate(&wrapped)
    }

    fn outline(&self, content: &str) -> Result<String> {
        let forms = self.forms(content);
        if forms.is_empty() && !content.trim().is_empty() {
            tu::validate_treesit(content, self.has_parse_errors(content))?;
        }
        tu::outline_lines(content, &forms)
    }

    fn tree_depth(&self, content: &str, depth: usize) -> Result<String> {
        let Some(tree) = self.parse(content) else {
            return Ok(String::new());
        };
        Ok(tu::recursive_outline(content, tree.root_node(), depth))
    }

    fn node_bounds(&self, content: &str, symbol: &str) -> Result<(usize, usize)> {
        let target = symbol.trim();
        tu::bounds_in_forms(&self.forms(content), target)
            .ok_or_else(|| Error::FormNotFound(target.to_string()))
    }

    fn semantic_check(&self, content: &str) -> Vec<String> {
        let forms = self.forms(content);
        self.spec.semantic_check(content, &forms)
    }

    fn is_known_global(&self, name: &str) -> bool {
        self.spec.is_known_global(name)
    }

    fn find_errors(&self, content: &str) -> Vec<String> {
        self.parse(content)
            .map(|t| tu::find_error_nodes(content, t.root_node()))
            .unwrap_or_default()
    }

    fn form_body_range(&self, form_text: &str) -> Option<(usize, usize)> {
        let tree = self.parse(form_text)?;
        let info = tu::analyze_def_form(form_text, tree.root_node())?;
        Some((info.body_start, info.body_end))
    }

    fn form_params_and_body(&self, form_text: &str) -> Option<(Vec<String>, String)> {
        let tree = self.parse(form_text)?;
        let info = tu::analyze_def_form(form_text, tree.root_node())?;
        Some((
            info.param_names,
            form_text[info.body_start..info.body_end].to_string(),
        ))
    }

    fn form_rename_name(&self, form_text: &str, old: &str, new: &str) -> Option<String> {
        let tree = self.parse(form_text)?;
        let info = tu::analyze_def_form(form_text, tree.root_node())?;
        if &form_text[info.name_start..info.name_end] != old {
            return None;
        }
        let mut result = form_text.to_string();
        result.replace_range(info.name_start..info.name_end, new);
        Some(result)
    }

    fn find_sexp_in(&self, content: &str, pattern: &str) -> Option<Option<(usize, usize)>> {
        let tree = self.parse(content)?;
        Some(tu::find_sexp_in_tree(content, pattern, tree.root_node()))
    }

    fn find_symbol_refs(&self, content: &str, symbol: &str) -> Vec<SymbolRef> {
        self.parse(content)
            .map(|t| tu::find_symbol_refs_in_tree(content, t.root_node(), symbol))
            .unwrap_or_default()
    }

    fn referenced_names(&self, content: &str) -> HashSet<String> {
        self.parse(content)
            .map(|t| tu::referenced_names_in_tree(content, t.root_node()))
            .unwrap_or_default()
    }
}

/// Per-thread, per-language parse cache: a reusable `Parser` (so we don't
/// rebuild it and re-run `set_language`) plus a memo of the most recent
/// `(content, tree)`. A single logical edit calls several plugin methods on the
/// same unchanged buffer (e.g. `check_file` then `node_bounds`); the memo lets
/// those reuse one parse instead of re-parsing the identical content each time.
struct ParserCache {
    parser: Parser,
    last: Option<(String, Tree)>,
}

/// Parse `content` for `language`, returning the memoized tree when `content`
/// matches the last parse for this language on this thread.
fn parse_cached(id: &'static str, language: &Language, content: &str) -> Option<Tree> {
    thread_local! {
        static CACHES: RefCell<HashMap<&'static str, ParserCache>> = RefCell::new(HashMap::new());
    }
    CACHES.with(|cell| {
        let mut map = cell.borrow_mut();
        let cache = map.entry(id).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(language)
                .expect("tree-sitter set_language failed");
            ParserCache {
                parser: p,
                last: None,
            }
        });
        if let Some((last_content, tree)) = &cache.last {
            if last_content == content {
                return Some(tree.clone());
            }
        }
        let tree = cache.parser.parse(content, None)?;
        cache.last = Some((content.to_string(), tree.clone()));
        Some(tree)
    })
}
