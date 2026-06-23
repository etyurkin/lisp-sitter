use std::collections::HashMap;
use std::path::Path;

use lisp_sitter_core::Registry;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Config {
    #[serde(default)]
    pub extensions: HashMap<String, String>,
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

    /// Register custom extension mappings into the Registry.
    pub fn apply(&self, reg: &mut Registry) {
        for (ext, lang_id) in &self.extensions {
            let ext = if ext.starts_with('.') { ext.clone() } else { format!(".{ext}") };
            reg.add_extension(&ext, lang_id);
        }
    }
}
