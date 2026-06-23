#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FormInfo {
    pub name: Option<String>,
    pub label: String,
    pub start: usize,
    pub end: usize,
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
        if depth <= 1 { self.outline(content) } else { Err(crate::Error::NotImplemented("tree_depth".into())) }
    }
    fn node_bounds(&self, content: &str, symbol: &str) -> crate::Result<(usize, usize)>;
    fn semantic_check(&self, content: &str) -> Vec<String> { let _ = content; Vec::new() }
}
