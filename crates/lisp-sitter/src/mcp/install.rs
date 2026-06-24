use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn install_config(cursor: bool, claude: bool) -> Result<()> {
    let binary = std::env::current_exe().context("resolve lisp-sitter binary path")?;
    let entry = json!({
        "type": "stdio",
        "command": binary,
        "args": ["mcp", "serve"],
        "env": {}
    });

    if cursor || !claude {
        merge_mcp_file(&cursor_config_path()?, "lisp-sitter", &entry)?;
        eprintln!("Updated {}", cursor_config_path()?.display());
    }
    if claude {
        // Claude Code CLI reads MCP servers from ~/.claude.json (user scope).
        // Claude Desktop uses ~/.claude/settings.json; write to both.
        merge_mcp_file(&claude_code_path()?, "lisp-sitter", &entry)?;
        eprintln!("Updated {}", claude_code_path()?.display());
        merge_mcp_file(&claude_desktop_path()?, "lisp-sitter", &entry)?;
        eprintln!("Updated {}", claude_desktop_path()?.display());
    }
    Ok(())
}

fn cursor_config_path() -> Result<PathBuf> {
    Ok(dirs_home()?.join(".cursor").join("mcp.json"))
}

fn claude_code_path() -> Result<PathBuf> {
    Ok(dirs_home()?.join(".claude.json"))
}

fn claude_desktop_path() -> Result<PathBuf> {
    Ok(dirs_home()?.join(".claude").join("settings.json"))
}

fn dirs_home() -> Result<PathBuf> {
    // LISP_SITTER_HOME overrides HOME for testing
    std::env::var_os("LISP_SITTER_HOME")
        .or_else(|| std::env::var_os("HOME"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    fn env_serial() -> &'static Mutex<()> {
        static MU: OnceLock<Mutex<()>> = OnceLock::new();
        MU.get_or_init(|| Mutex::new(()))
    }

    fn test_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lisp-sitter-install-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_config_paths() {
        let home = dirs_home().unwrap();
        assert_eq!(cursor_config_path().unwrap(), home.join(".cursor").join("mcp.json"));
        assert_eq!(claude_code_path().unwrap(), home.join(".claude.json"));
        assert_eq!(claude_desktop_path().unwrap(), home.join(".claude").join("settings.json"));
    }

    #[test]
    fn test_merge_mcp_file_creates_new_file() {
        let dir = test_dir("creates_new");
        let path = dir.join("test-config.json");
        let entry = json!({"type": "stdio", "command": "/bin/test", "args": [], "env": {}});

        merge_mcp_file(&path, "my-server", &entry).unwrap();
        assert!(path.exists());

        let content: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["mcpServers"]["my-server"]["command"], "/bin/test");
        assert_eq!(content["mcpServers"]["my-server"]["type"], "stdio");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_merge_mcp_file_merges_existing() {
        let dir = test_dir("merges_existing");
        let path = dir.join("test-config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"existing":{"command":"/old"}}}"#).unwrap();

        let entry = json!({"command": "/new", "args": []});
        merge_mcp_file(&path, "new-server", &entry).unwrap();

        let content: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(content["mcpServers"].as_object().unwrap().contains_key("existing"));
        assert!(content["mcpServers"].as_object().unwrap().contains_key("new-server"));
        assert_eq!(content["mcpServers"]["new-server"]["command"], "/new");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_merge_mcp_file_overwrites_same_name() {
        let dir = test_dir("overwrites");
        let path = dir.join("test-config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"srv":{"command":"/old","args":[]}}}"#).unwrap();

        let entry = json!({"command": "/new", "args": ["serve"], "type": "stdio"});
        merge_mcp_file(&path, "srv", &entry).unwrap();

        let content: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["mcpServers"]["srv"]["command"], "/new");
        assert_eq!(content["mcpServers"]["srv"]["type"], "stdio");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_entry_format_includes_type_and_env() {
        let binary = std::env::current_exe().unwrap();
        let entry = json!({
            "type": "stdio",
            "command": binary,
            "args": ["mcp", "serve"],
            "env": {}
        });
        assert_eq!(entry["type"], "stdio");
        assert_eq!(entry["args"], json!(["mcp", "serve"]));
        assert!(entry["env"].as_object().unwrap().is_empty());
    }

    #[test]
    fn test_install_config_writes_files() {
        let _guard = env_serial().lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lisp-sitter-install-test-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LISP_SITTER_HOME", dir.to_str().unwrap());
        install_config(true, true).unwrap();
        assert!(dir.join(".cursor").join("mcp.json").exists());
        assert!(dir.join(".claude.json").exists());
        assert!(dir.join(".claude").join("settings.json").exists());
        let content: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(".claude.json")).unwrap()
        ).unwrap();
        assert_eq!(content["mcpServers"]["lisp-sitter"]["type"], "stdio");
        assert!(content["mcpServers"]["lisp-sitter"]["command"].as_str().unwrap().contains("lisp-sitter"));
        std::env::remove_var("LISP_SITTER_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_install_config_default_cursor() {
        let _guard = env_serial().lock().unwrap();
        let dir = std::env::temp_dir().join(format!("lisp-sitter-install-test-default-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LISP_SITTER_HOME", dir.to_str().unwrap());
        install_config(false, false).unwrap();
        assert!(dir.join(".cursor").join("mcp.json").exists());
        assert!(!dir.join(".claude.json").exists());
        std::env::remove_var("LISP_SITTER_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
