use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("no plugin for path: {0}")]
    NoPlugin(String),

    #[error("no top-level form `{0}` found")]
    FormNotFound(String),

    #[error("{0} is only for new/empty files; use {1} or a symbol name")]
    StartAnchorOnNonempty(String, String),

    #[error("new_body must be a complete top-level sexp")]
    BodyNotSexp,

    #[error("form must not be empty")]
    EmptyForm,

    #[error("SYNTAX ERROR — {0}")]
    Syntax(String),

    #[error("source file is malformed; refusing to edit (bounds would be unreliable and could delete following forms) — {0}\n\nFix the imbalance first, then retry.")]
    MalformedSource(String),

    #[error("SYNTAX ERROR after {operation} — {detail}")]
    SyntaxAfterEdit { operation: String, detail: String },

    #[error("{0}")]
    Message(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn check_ok() -> String {
    "OK".to_string()
}

pub fn syntax_error(detail: impl Into<String>) -> String {
    format!(
        "SYNTAX ERROR — {}\n\nFix the file and call check before writing.",
        detail.into()
    )
}

pub fn syntax_error_node(detail: impl Into<String>) -> String {
    format!(
        "SYNTAX ERROR — {}\n\nFix the form and call check-node again.",
        detail.into()
    )
}
