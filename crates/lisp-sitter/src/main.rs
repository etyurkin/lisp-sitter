mod commands;
mod mcp;

use std::io::{Read, Write};

use anyhow::Result;
use clap::CommandFactory;
use clap::{Parser, Subcommand};

use lisp_sitter::default_registry;

#[derive(Parser)]
#[command(name = "lisp-sitter", version, about = "Structural editing for Emacs Lisp, Scheme, and Common Lisp")]
struct Cli {
    #[command(subcommand)] command: Command,
    #[arg(long, global = true, help = "Language override: elisp, commonlisp, scheme (or set LISP_SITTER_LANG env var)")] lang: Option<String>,
    #[arg(long, global = true, help = "Output machine-readable JSON")] json: bool,
    #[arg(long, global = true, help = "Show diff and prompt before writing")] confirm: bool,
}

#[derive(Subcommand)]
enum Command {
    /// List top-level forms in a file.
    /// Examples:
    ///   lisp-sitter tree foo.el
    ///   lisp-sitter tree foo.el --depth 2
    ///   lisp-sitter tree "src/**/*.el" --batch
    Tree { path: String, #[arg(long, default_value = "1")] depth: usize },
    /// Return byte positions START:END for a named top-level form.
    /// Example: lisp-sitter bounds foo.el my-function
    Bounds { path: String, symbol: String },
    /// Replace a top-level form with new complete text.
    /// Examples:
    ///   lisp-sitter replace foo.el my-func --body '(defun my-func () 42)' --write
    ///   echo '(defun x () 1)' | lisp-sitter replace foo.el old --body-file - --write
    Replace { path: String, symbol: String, #[arg(long, conflicts_with = "body_file")] body: Option<String>, #[arg(long)] body_file: Option<String>, #[arg(long)] write: bool, #[arg(long)] diff: bool },
    /// Insert a top-level form after __start__, __end__, or a symbol name.
    /// Examples:
    ///   lisp-sitter insert foo.el my-func --node '(defun helper () t)' --write
    ///   lisp-sitter insert new.scm __start__ --node '(define version 1)' --write
    Insert { path: String, after: String, #[arg(long, conflicts_with = "node_file")] node: Option<String>, #[arg(long)] node_file: Option<String>, #[arg(long)] write: bool, #[arg(long)] diff: bool },
    /// Validate a whole file. Pass --semantic for deep checks (docstrings, provides).
    /// Example: lisp-sitter check foo.el --semantic
    Check { path: String, #[arg(long)] semantic: bool },
    /// Print the full text of a named top-level form.
    /// Example: lisp-sitter get foo.el my-function
    Get { path: String, symbol: String },
    /// Complete missing closing parens of an s-expression.
    /// Example: lisp-sitter complete --lang elisp --body '(defun foo (x) (if x 1'
    Complete { #[arg(long, default_value = "elisp")] lang: String, #[arg(long, conflicts_with = "body_file")] body: Option<String>, #[arg(long)] body_file: Option<String> },
    /// Re-indent a file (depth-based, 2-space indent).
    /// Example: lisp-sitter fmt foo.el --write
    Fmt { path: String, #[arg(long)] write: bool, #[arg(long)] diff: bool },
    /// Evaluate using native tool (emacs byte-compile, sbcl --script, guile -s).
    /// Example: lisp-sitter eval foo.el
    Eval { path: String },
    /// Rename a top-level form and its call sites.
    /// Example: lisp-sitter rename foo.el my-func my-new-func --write
    Rename { path: String, old: String, new: String, #[arg(long)] write: bool },
    /// Wrap the body of a form in a construct (progn, let, if).
    /// Examples:
    ///   lisp-sitter wrap foo.el my-func --in let --bindings '((x 1))' --write
    ///   lisp-sitter wrap foo.el my-func --in if --condition '(> x 0)' --write
    Wrap { path: String, symbol: String, #[arg(long)] r#in: String, #[arg(long)] bindings: Option<String>, #[arg(long)] condition: Option<String>, #[arg(long)] write: bool },
    /// Remove a top-level form. Use --keep-calls to leave call sites as-is.
    /// Example: lisp-sitter remove foo.el dead-func --write
    Remove { path: String, symbol: String, #[arg(long)] keep_calls: bool, #[arg(long)] write: bool },
    /// Move a top-level form after another symbol, __start__, or __end__.
    /// Example: lisp-sitter move foo.el my-func --after other-func --write
    Move { path: String, symbol: String, after: String, #[arg(long)] write: bool },
    /// Replace a sub-expression inside a named form.
    /// Example: lisp-sitter substitute foo.el my-func --pattern '(> x 0)' --replacement '(>= x 0)' --write
    Substitute { path: String, symbol: String, #[arg(long)] pattern: String, #[arg(long)] replacement: String, #[arg(long)] write: bool },
    /// Extract a sub-expression into a new function.
    /// Example: lisp-sitter extract foo.el my-func --pattern '(* x x)' --name square --write
    Extract { path: String, symbol: String, #[arg(long)] pattern: String, #[arg(long)] name: String, #[arg(long)] params: Option<String>, #[arg(long)] write: bool },
    /// Find all callers of a symbol in a file.
    /// Example: lisp-sitter callers foo.el my-func
    Callers { path: String, symbol: String },
    /// Install a git pre-commit hook that runs lisp-sitter check on Lisp files.
    /// Example: lisp-sitter init-git-hook
    InitGitHook,
    /// Instrument a form's body with tracing (--with for body, --at/--wrap for sub-expressions).
    /// Examples:
    ///   lisp-sitter instrument foo.el my-func --with '(message "trace")' --write
    ///   lisp-sitter instrument foo.el my-func --at '(compute x)' --wrap '(progn (msg) <form>)' --write
    Instrument { path: String, symbol: String, #[arg(long)] r#with: Option<String>, #[arg(long)] at: Option<String>, #[arg(long)] wrap: Option<String>, #[arg(long)] write: bool },
    /// Inline a single-call-site function body at its call site and remove the definition.
    /// Example: lisp-sitter flatten foo.el helper --write
    Flatten { path: String, symbol: String, #[arg(long)] write: bool },
    /// Convert between let and let* bindings.
    /// Example: lisp-sitter convert-let foo.el my-func --to let* --write
    ConvertLet { path: String, symbol: String, #[arg(long)] to: String, #[arg(long)] write: bool },
    /// Validate a single top-level form without saving.
    /// Example: lisp-sitter check-node --lang scheme --body '(define x 1)'
    CheckNode { #[arg(long, default_value = "elisp")] lang: String, #[arg(long, conflicts_with = "body_file")] body: Option<String>, #[arg(long)] body_file: Option<String> },
    /// Generate shell completion scripts.
    /// Usage: eval "$(lisp-sitter completions bash)"  # or zsh, fish, powershell, elvish
    Completions { shell: String },
    /// Model Context Protocol server for AI agent integration.
    /// Subcommands: serve (stdio transport), install (add to Cursor/Claude config)
    Mcp { #[command(subcommand)] command: McpCommand },
}

#[derive(Subcommand)]
enum McpCommand {
    /// Run MCP server on stdio (for Cursor, Claude Code, etc.)
    Serve,
    /// Write MCP config to ~/.cursor/mcp.json and/or ~/.claude.json + ~/.claude/settings.json
    Install { #[arg(long)] cursor: bool, #[arg(long, help = "Write to Claude Code (~/.claude.json) and Claude Desktop (~/.claude/settings.json)")] claude: bool },
}

fn confirm_or_abort() {
    eprint!("Apply? [y/N] "); let _ = std::io::stderr().flush();
    let mut buf = String::new(); let _ = std::io::stdin().read_line(&mut buf);
    if !buf.trim().eq_ignore_ascii_case("y") { std::process::exit(0); }
}

fn read_text(inline: Option<String>, file: Option<String>) -> Result<String> {
    match (inline, file) { (Some(t), None) => Ok(t), (None, Some(p)) => { if p == "-" { let mut buf = String::new(); std::io::stdin().read_to_string(&mut buf)?; Ok(buf) } else { Ok(std::fs::read_to_string(p)?) } } (None, None) => anyhow::bail!("provide --body/--node text or --body-file/--node-file"), (Some(_), Some(_)) => unreachable!("clap conflicts_with") }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(ref lang) = cli.lang { std::env::set_var("LISP_SITTER_LANG", lang); }
    let reg = default_registry();
    let j = cli.json; let cf = cli.confirm;
    match cli.command {
        Command::Tree { path, depth } => {
            if j { let c = lisp_sitter::ops::read_file(&path)?; let p = lisp_sitter::ops::resolve_plugin(&reg, &path, None)?; println!("{}", serde_json::to_string_pretty(&p.list_forms(&c)?)?); }
            else if depth > 1 { println!("{}", lisp_sitter::ops::tree_depth(&reg, &path, depth)?); } else { println!("{}", lisp_sitter::ops::tree(&reg, &path)?); }
        }
        Command::Bounds { ref path, ref symbol } => { println!("{}", lisp_sitter::ops::bounds(&reg, path, symbol)?); }
        Command::Replace { path, symbol, body, body_file, write, diff } => {
            let b = read_text(body, body_file)?; let content = lisp_sitter::ops::read_file(&path)?;
            let u = lisp_sitter_core::edit::replace_node(reg.plugin_for_path(&path)?, &content, &symbol, &b)?;
            if diff || cf { let d = lisp_sitter::ops::diff_text(&content, &u, &path); if !d.is_empty() { eprint!("{d}"); } }
            if cf { confirm_or_abort(); } if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); }
        }
        Command::Insert { path, after, node, node_file, write, diff } => {
            let n = read_text(node, node_file)?; let content = lisp_sitter::ops::read_file(&path)?;
            let u = lisp_sitter_core::edit::insert_after(reg.plugin_for_path(&path)?, &content, &after, &n)?;
            if diff || cf { let d = lisp_sitter::ops::diff_text(&content, &u, &path); if !d.is_empty() { eprint!("{d}"); } }
            if cf { confirm_or_abort(); } if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); }
        }
        Command::Check { path, semantic } => { if semantic { commands::check_semantic(&reg, &path)?; } else { commands::check(&reg, &path)?; } }
        Command::Get { path, symbol } => { print!("{}", lisp_sitter::ops::get_form(&reg, &path, &symbol)?); }
        Command::Complete { lang, body, body_file } => { println!("{}", lisp_sitter::ops::complete_node(&reg, &lang, &read_text(body, body_file)?)?); }
        Command::Fmt { path, write, diff } => {
            let content = lisp_sitter::ops::read_file(&path)?; let f = lisp_sitter_core::format_source(&content);
            if diff || cf { let d = lisp_sitter::ops::diff_text(&content, &f, &path); if !d.is_empty() { eprint!("{d}"); } }
            if cf { confirm_or_abort(); } if write { reg.plugin_for_path(&path)?.check_file(&f)?; lisp_sitter::ops::atomic_write(&path, &f)?; println!("Wrote {path}"); } else { print!("{f}"); }
        }
        Command::Eval { path } => { commands::eval(&path)?; }
        Command::Rename { path, old, new, write } => {
            let u = lisp_sitter::transform::rename(&reg, &path, &old, &new)?;
            if cf { let d = lisp_sitter::ops::diff_text(&lisp_sitter::ops::read_file(&path)?, &u, &path); if !d.is_empty() { eprint!("{d}"); } confirm_or_abort(); }
            if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); }
        }
        Command::Wrap { path, symbol, r#in, bindings, condition, write } => { commands::wrap(&reg, &path, &symbol, &r#in, bindings.as_deref(), condition.as_deref(), write)?; }
        Command::Remove { path, symbol, keep_calls, write } => { let u = lisp_sitter::transform::remove_form(&reg, &path, &symbol, keep_calls)?; if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); } }
        Command::Move { path, symbol, after, write } => { let u = lisp_sitter::transform::move_form(&reg, &path, &symbol, &after)?; if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); } }
        Command::Substitute { path, symbol, pattern, replacement, write } => { let u = lisp_sitter::transform::substitute(&reg, &path, &symbol, &pattern, &replacement)?; if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); } }
        Command::Extract { path, symbol, pattern, name, params, write } => { let p: Vec<&str> = params.as_deref().unwrap_or("").split(',').filter(|s| !s.is_empty()).collect(); let u = lisp_sitter::transform::extract(&reg, &path, &symbol, &pattern, &name, &p)?; if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); } }
        Command::Callers { path, symbol } => {
            if j { let c = lisp_sitter::ops::read_file(&path)?; let p = lisp_sitter::ops::resolve_plugin(&reg, &path, None)?; let def = p.node_bounds(&c, &symbol).ok();
                let pat = format!("({} ", symbol); let pat2 = format!("({})", symbol); let mut r = Vec::new(); let mut s = 0;
                loop { let pos = if let Some(pos) = c[s..].find(&pat) { s + pos } else if let Some(pos) = c[s..].find(&pat2) { s + pos } else { break };
                    if def.map_or(false, |(ds, de)| pos >= ds && pos < de) { s = pos + 1; continue; }
                    if let Some(o) = p.list_forms(&c)?.iter().find(|f| pos >= f.start && pos < f.end) { r.push(serde_json::json!({"in": o.label, "label": format!("{} calls {}", o.label, symbol), "start": pos})); }
                    s = pos + 1; }
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else { println!("{}", lisp_sitter::ops::callers(&reg, &path, &symbol)?); }
        }
        Command::InitGitHook => { commands::init_git_hook()?; }
        Command::Instrument { path, symbol, r#with, at, wrap, write } => { let u = lisp_sitter::transform::instrument(&reg, &path, &symbol, r#with.as_deref(), at.as_deref(), wrap.as_deref())?; if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); } }
        Command::Flatten { path, symbol, write } => { let u = lisp_sitter::transform::flatten(&reg, &path, &symbol)?; if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); } }
        Command::ConvertLet { path, symbol, to, write } => { let u = lisp_sitter::transform::convert_let(&reg, &path, &symbol, &to)?; if write { lisp_sitter::ops::atomic_write(&path, &u)?; println!("Wrote {path}"); } else { print!("{u}"); } }
        Command::CheckNode { lang, body, body_file } => { println!("{}", lisp_sitter::ops::check_node_by_lang(&reg, &lang, &read_text(body, body_file)?)?); }
        Command::Completions { shell } => { let s: clap_complete::Shell = shell.parse().map_err(|e| anyhow::anyhow!("unknown shell: {e}"))?; let mut cmd = Cli::command(); clap_complete::generate(s, &mut cmd, "lisp-sitter", &mut std::io::stdout()); }
        Command::Mcp { command } => match command { McpCommand::Serve => { mcp::serve_stdio(reg).await?; }, McpCommand::Install { cursor, claude } => { mcp::install_config(cursor, claude)?; } },
    };
    Ok(())
}
