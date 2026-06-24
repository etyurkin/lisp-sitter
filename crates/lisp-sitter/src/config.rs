use std::collections::HashMap;
use std::path::Path;

use lisp_sitter_core::Registry;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Config {
    #[serde(default)]
    pub extensions: HashMap<String, String>,
    /// Extra top-level definer keywords per language id (`elisp`, `commonlisp`,
    /// `scheme`) — e.g. project-specific def-macros. Each is treated like a
    /// `defun`/`define` (name is the second element).
    #[serde(default)]
    pub extra_definers: HashMap<String, Vec<String>>,
}

impl Config {
    /// Load config from standard locations (~/.config/lisp-sitter/config.json or ~/.lisp-sitter.json).
    pub fn load() -> Self {
        let candidates = [
            std::env::var("HOME").ok().map(|h| format!("{h}/.config/lisp-sitter/config.json")),
            std::env::var("HOME").ok().map(|h| format!("{h}/.lisp-sitter.json")),
            std::env::var("LISP_SITTER_CONFIG").ok(),
        ];
        for path in candidates.into_iter().flatten() {
            let p = Path::new(&path);
            if p.exists() {
                if let Ok(content) = std::fs::read_to_string(p) {
                    if let Ok(cfg) = serde_json::from_str::<Config>(&content) {
                        return cfg;
                    }
                }
            }
        }
        Config::default()
    }

    /// Extra definer keywords configured for a language id (empty if none).
    pub fn definers_for(&self, lang_id: &str) -> &[String] {
        self.extra_definers.get(lang_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Register custom extension mappings into the Registry.
    pub fn apply(&self, reg: &mut Registry) {
        for (ext, lang_id) in &self.extensions {
            let ext = if ext.starts_with('.') { ext.clone() } else { format!(".{ext}") };
            reg.add_extension(&ext, lang_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use std::sync::Mutex;

    /// Serialize tests that modify the `LISP_SITTER_CONFIG` env var.
    fn env_serial() -> &'static Mutex<()> {
        static MU: OnceLock<Mutex<()>> = OnceLock::new();
        MU.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn test_load_default_when_no_config() {
        let cfg = Config::load();
        assert!(cfg.extensions.is_empty());
    }

    #[test]
    fn test_apply_adds_extension() {
        let mut reg = Registry::default();
        let mut cfg = Config::default();
        cfg.extensions.insert("foo".to_string(), "elisp".to_string());
        cfg.apply(&mut reg);
    }

    #[test]
    fn test_load_from_config_file() {
        let _guard = env_serial().lock().unwrap();
        let cfg_key = "LISP_SITTER_CONFIG";
        let old = std::env::var(cfg_key).ok();
        let dir = std::env::temp_dir().join(format!("lisp-sitter-config-test-{}-1", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("test-config.json").to_str().unwrap().to_string();
        std::fs::write(&config_path, r#"{"extensions":{"wl":"elisp"}}"#).unwrap();
        std::env::set_var(cfg_key, &config_path);
        let cfg = Config::load();
        assert_eq!(cfg.extensions.get("wl").map(|s| s.as_str()), Some("elisp"));
        match old { Some(v) => std::env::set_var(cfg_key, v), None => std::env::remove_var(cfg_key) }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_invalid_json_falls_back() {
        let _guard = env_serial().lock().unwrap();
        let cfg_key = "LISP_SITTER_CONFIG";
        let old = std::env::var(cfg_key).ok();
        let dir = std::env::temp_dir().join(format!("lisp-sitter-config-test-{}-2", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("bad.json").to_str().unwrap().to_string();
        std::fs::write(&config_path, "not valid json").unwrap();
        std::env::set_var(cfg_key, &config_path);
        let cfg = Config::load();
        assert!(cfg.extensions.is_empty(), "expected empty config, got {:?}", cfg.extensions);
        match old { Some(v) => std::env::set_var(cfg_key, v), None => std::env::remove_var(cfg_key) }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extension_normalization() {
        let mut cfg = Config::default();
        cfg.extensions.insert(".ext".to_string(), "elisp".to_string());
        cfg.extensions.insert("bare".to_string(), "scheme".to_string());
        // just verify it doesn't panic
        let mut reg = Registry::default();
        cfg.apply(&mut reg);
    }
}
