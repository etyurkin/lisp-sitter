use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn install_config(cursor: bool, claude: bool) -> Result<()> {
    let binary = std::env::current_exe().context("resolve lisp-sitter binary path")?;
    let entry = json!({
        "command": binary,
        "args": ["mcp", "serve"]
    });

    if cursor || (!cursor && !claude) {
        merge_mcp_file(&cursor_config_path()?, "lisp-sitter", &entry)?;
        eprintln!("Updated {}", cursor_config_path()?.display());
    }
    if claude {
        merge_mcp_file(&claude_config_path()?, "lisp-sitter", &entry)?;
        eprintln!("Updated {}", claude_config_path()?.display());
    }
    Ok(())
}

fn cursor_config_path() -> Result<PathBuf> {
    Ok(dirs_home()?.join(".cursor").join("mcp.json"))
}

fn claude_config_path() -> Result<PathBuf> {
    Ok(dirs_home()?.join(".claude").join("settings.json"))
}

fn dirs_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("$HOME is not set")
}

fn merge_mcp_file(path: &Path, name: &str, entry: &Value) -> Result<()> {
    let mut root: Value = if path.exists() {
        let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&text).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    let servers = root
        .as_object_mut()
        .context("config root must be object")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));

    servers
        .as_object_mut()
        .context("mcpServers must be object")?
        .insert(name.to_string(), entry.clone());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let out = serde_json::to_string_pretty(&root)?;
    std::fs::write(path, out).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
