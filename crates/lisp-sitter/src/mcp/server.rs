use std::sync::Arc;

use lisp_sitter::ops;
use lisp_sitter_core::Registry;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use rmcp::transport::stdio;
use serde::Deserialize;

#[derive(Clone)]
pub struct LispSitterMcp {
    reg: Arc<Registry>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl LispSitterMcp { pub fn new(reg: Registry) -> Self { Self { reg: Arc::new(reg), tool_router: Self::tool_router() } } }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathArgs { path: String, #[serde(default)] semantic: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathDepthArgs { path: String, #[serde(default)] #[allow(dead_code)] depth: Option<i64> }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathSymbolArgs { path: String, symbol: String }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathNodeArgs { path: String, node: String }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReplaceArgs { path: String, symbol: String, new_body: String, #[serde(default)] diff: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InsertArgs { path: String, after_symbol: String, node: String, #[serde(default)] diff: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CompleteArgs { lang: String, body: String }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormatArgs { path: String, #[serde(default)] write: bool, #[serde(default)] diff: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RenameArgs { path: String, old: String, new: String, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WrapArgs { path: String, symbol: String, r#in: String, #[serde(default)] bindings: Option<String>, #[serde(default)] condition: Option<String>, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RemoveArgs { path: String, symbol: String, #[serde(default)] keep_calls: bool, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MoveArgs { path: String, symbol: String, after: String, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SubstituteArgs { path: String, symbol: String, pattern: String, replacement: String, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExtractArgs { path: String, symbol: String, pattern: String, name: String, #[serde(default)] params: Option<String>, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FlattenArgs { path: String, symbol: String, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConvertLetArgs { path: String, symbol: String, to: String, #[serde(default)] write: bool }

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InstrumentArgs { path: String, symbol: String, #[serde(default)] #[schemars(description = "Tracing form")] r#with: Option<String>, #[serde(default)] #[schemars(description = "Sub-expression")] at: Option<String>, #[serde(default)] #[schemars(description = "Wrapper")] wrap: Option<String>, #[serde(default)] write: bool }

fn tool_result(r: Result<String, lisp_sitter_core::Error>) -> Result<String, String> { match r { Ok(t) => Ok(t), Err(e) => Err(e.to_string()) } }

#[tool_router]
impl LispSitterMcp {

    #[tool(description = "Validate a whole structural Lisp file. Pass semantic=true for deep validation.")]
    async fn check_structural_file(&self, Parameters(args): Parameters<PathArgs>) -> Result<String, String> {
        if args.semantic { tool_result(ops::check_semantic(&self.reg, &args.path)) } else { tool_result(ops::check_structural_file(&self.reg, &args.path)) }
    }

    #[tool(description = "Validate a complete top-level node without saving.")]
    async fn check_structural_node(&self, Parameters(args): Parameters<PathNodeArgs>) -> Result<String, String> { tool_result(ops::check_structural_node(&self.reg, &args.path, &args.node)) }

    #[tool(description = "Outline of top-level forms. Set depth > 1 for sub-form navigation.")]
    async fn structural_tree(&self, Parameters(args): Parameters<PathDepthArgs>) -> Result<String, String> {
        let d = args.depth.unwrap_or(1) as usize;
        if d > 1 { tool_result(ops::tree_depth(&self.reg, &args.path, d)) } else { tool_result(ops::tree(&self.reg, &args.path)) }
    }

    #[tool(description = "Return byte positions START:END for a named top-level form.")]
    async fn structural_bounds(&self, Parameters(args): Parameters<PathSymbolArgs>) -> Result<String, String> { tool_result(ops::bounds(&self.reg, &args.path, &args.symbol)) }

    #[tool(description = "Replace one top-level form with complete new text. Validates and saves.")]
    async fn structural_replace(&self, Parameters(args): Parameters<ReplaceArgs>) -> Result<String, String> {
        if args.diff { let c = ops::read_file(&args.path).map_err(|e| e.to_string())?; let p = self.reg.plugin_for_path(&args.path).map_err(|e| e.to_string())?;
            let u = lisp_sitter_core::edit::replace_node(p, &c, &args.symbol, &args.new_body).map_err(|e| e.to_string())?;
            ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}\n{}", args.path, ops::diff_text(&c, &u, &args.path))) }
        else { tool_result(ops::replace(&self.reg, &args.path, &args.symbol, &args.new_body)) }
    }

    #[tool(description = "Insert a complete top-level form after __start__, __end__, or a symbol name.")]
    async fn structural_insert(&self, Parameters(args): Parameters<InsertArgs>) -> Result<String, String> {
        if args.diff { let c = ops::read_file(&args.path).map_err(|e| e.to_string())?; let p = self.reg.plugin_for_path(&args.path).map_err(|e| e.to_string())?;
            let u = lisp_sitter_core::edit::insert_after(p, &c, &args.after_symbol, &args.node).map_err(|e| e.to_string())?;
            ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}\n{}", args.path, ops::diff_text(&c, &u, &args.path))) }
        else { tool_result(ops::insert(&self.reg, &args.path, &args.after_symbol, &args.node)) }
    }

    #[tool(description = "Return the full text of a named top-level form.")]
    async fn structural_get(&self, Parameters(args): Parameters<PathSymbolArgs>) -> Result<String, String> { tool_result(ops::get_form(&self.reg, &args.path, &args.symbol)) }

    #[tool(description = "Complete an unbalanced s-expression by appending missing closing parens.")]
    async fn structural_complete(&self, Parameters(args): Parameters<CompleteArgs>) -> Result<String, String> { tool_result(ops::complete_node(&self.reg, &args.lang, &args.body)) }

    #[tool(description = "Return the complete structural context: outline, bounds, and full text.")]
    async fn structural_context(&self, Parameters(args): Parameters<PathArgs>) -> Result<String, String> { tool_result(ops::context(&self.reg, &args.path)) }

    #[tool(description = "Evaluate a file using the language's native tool (emacs, sbcl, guile).")]
    async fn structural_eval(&self, Parameters(args): Parameters<PathArgs>) -> Result<String, String> {
        match lisp_sitter::eval::eval_file(&args.path) { Ok((s, e, ok)) => { let mut r = String::new(); if !s.is_empty() { r.push_str(&s); } if !e.is_empty() { if !r.is_empty() { r.push('\n'); } r.push_str(&e); } if ok { Ok(r) } else { Err(if r.is_empty() { "error".into() } else { r }) } } Err(e) => Err(e.to_string()) }
    }

    #[tool(description = "Re-indent a file (depth-based). Pass write=true to save.")]
    async fn structural_format(&self, Parameters(args): Parameters<FormatArgs>) -> Result<String, String> {
        let c = ops::read_file(&args.path).map_err(|e| e.to_string())?; let f = lisp_sitter_core::format_source(&c); let mut r = String::new();
        if args.diff { let d = ops::diff_text(&c, &f, &args.path); if !d.is_empty() { r.push_str(&d); } }
        if args.write { let p = self.reg.plugin_for_path(&args.path).map_err(|e| e.to_string())?; p.check_file(&f).map_err(|e| e.to_string())?; ops::atomic_write(&args.path, &f).map_err(|e| e.to_string())?; r.push_str(&format!("Wrote {}", args.path)); } else { r.push_str(&f); } Ok(r)
    }

    #[tool(description = "Rename a top-level form and its call sites.")]
    async fn structural_rename(&self, Parameters(args): Parameters<RenameArgs>) -> Result<String, String> {
        let u = lisp_sitter::transform::rename(&self.reg, &args.path, &args.old, &args.new).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Wrap the body of a named form in a construct (progn, let, if).")]
    async fn structural_wrap(&self, Parameters(args): Parameters<WrapArgs>) -> Result<String, String> {
        let mut xs: Vec<(&str, &str)> = Vec::new();
        if let Some(ref b) = args.bindings { xs.push(("bindings", b)); } if let Some(ref c) = args.condition { xs.push(("condition", c)); }
        let u = lisp_sitter::transform::wrap_body(&self.reg, &args.path, &args.symbol, &args.r#in, &xs).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Remove a top-level form (optionally call site stubs).")]
    async fn structural_remove(&self, Parameters(args): Parameters<RemoveArgs>) -> Result<String, String> {
        let u = lisp_sitter::transform::remove_form(&self.reg, &args.path, &args.symbol, args.keep_calls).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Move a top-level form after another symbol, __start__, or __end__.")]
    async fn structural_move(&self, Parameters(args): Parameters<MoveArgs>) -> Result<String, String> {
        let u = lisp_sitter::transform::move_form(&self.reg, &args.path, &args.symbol, &args.after).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Replace a sub-expression inside a named form.")]
    async fn structural_substitute(&self, Parameters(args): Parameters<SubstituteArgs>) -> Result<String, String> {
        let u = lisp_sitter::transform::substitute(&self.reg, &args.path, &args.symbol, &args.pattern, &args.replacement).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Extract a sub-expression into a new function.")]
    async fn structural_extract(&self, Parameters(args): Parameters<ExtractArgs>) -> Result<String, String> {
        let params: Vec<&str> = args.params.as_deref().unwrap_or("").split(',').filter(|s| !s.is_empty()).collect();
        let u = lisp_sitter::transform::extract(&self.reg, &args.path, &args.symbol, &args.pattern, &args.name, &params).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Find all callers of a symbol in a file.")]
    async fn structural_callers(&self, Parameters(args): Parameters<PathSymbolArgs>) -> Result<String, String> { tool_result(ops::callers(&self.reg, &args.path, &args.symbol)) }

    #[tool(description = "Instrument a form with tracing (--with for body, --at/--wrap for sub-expressions).")]
    async fn structural_instrument(&self, Parameters(args): Parameters<InstrumentArgs>) -> Result<String, String> {
        let u = lisp_sitter::transform::instrument(&self.reg, &args.path, &args.symbol, args.r#with.as_deref(), args.at.as_deref(), args.wrap.as_deref()).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Inline a single-call-site function body at its call site and remove the definition.")]
    async fn structural_flatten(&self, Parameters(args): Parameters<FlattenArgs>) -> Result<String, String> {
        let u = lisp_sitter::transform::flatten(&self.reg, &args.path, &args.symbol).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }

    #[tool(description = "Convert between let and let* bindings in a form.")]
    async fn structural_convert_let(&self, Parameters(args): Parameters<ConvertLetArgs>) -> Result<String, String> {
        let u = lisp_sitter::transform::convert_let(&self.reg, &args.path, &args.symbol, &args.to).map_err(|e| e.to_string())?;
        if args.write { ops::atomic_write(&args.path, &u).map_err(|e| e.to_string())?; Ok(format!("Wrote {}", args.path)) } else { Ok(u) }
    }
}

#[tool_handler]
impl ServerHandler for LispSitterMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Structural editing for Emacs Lisp, Scheme, and Common Lisp. \
             Use structural_tree → structural_bounds → structural_replace/structural_insert. \
             Each replace/insert must be a complete top-level form.",
        )
    }
}

pub async fn serve_stdio(reg: Registry) -> anyhow::Result<()> {
    let service = LispSitterMcp::new(reg).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
