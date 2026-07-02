use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::plugin::LanguagePlugin;

pub struct Registry {
    plugins: Vec<Box<dyn LanguagePlugin>>,
    extra_exts: HashMap<String, String>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            extra_exts: HashMap::new(),
        }
    }

    pub fn register(&mut self, plugin: Box<dyn LanguagePlugin>) {
        self.plugins.push(plugin);
    }

    /// Register a custom extension mapping (e.g. ".clef" -> "commonlisp").
    pub fn add_extension(&mut self, ext: &str, lang_id: &str) {
        let ext = if ext.starts_with('.') {
            ext.to_string()
        } else {
            format!(".{ext}")
        };
        self.extra_exts.insert(ext, lang_id.to_string());
    }

    pub fn plugin_for_path(&self, path: &str) -> Result<&dyn LanguagePlugin> {
        // Standard path-based matching first
        for p in &self.plugins {
            if p.matches_path(path) {
                return Ok(p.as_ref());
            }
        }
        // Custom extension mappings
        if let Some(lang_id) = self
            .extra_exts
            .iter()
            .find(|(ext, _)| path.ends_with(ext.as_str()))
            .map(|(_, id)| id)
        {
            for p in &self.plugins {
                if p.id() == lang_id.as_str() {
                    return Ok(p.as_ref());
                }
            }
            return Err(Error::NoPlugin(lang_id.to_string()));
        }
        Err(Error::NoPlugin(path.to_string()))
    }

    pub fn plugin_for_id(&self, id: &str) -> Option<&dyn LanguagePlugin> {
        self.plugins
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    pub fn plugins(&self) -> &[Box<dyn LanguagePlugin>] {
        &self.plugins
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
