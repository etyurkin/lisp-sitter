use crate::anchors::{is_anchor_end, is_anchor_start, ANCHOR_END, ANCHOR_START};
use crate::error::{Error, Result};
use crate::plugin::LanguagePlugin;
use crate::scan::{content_blank, replace_region};

pub fn get_form_text<'a>(
    plugin: &dyn LanguagePlugin,
    content: &'a str,
    symbol: &str,
) -> Result<&'a str> {
    let (start, end) = plugin.node_bounds(content, symbol)?;
    Ok(&content[start..end])
}

pub fn replace_node(
    plugin: &dyn LanguagePlugin,
    content: &str,
    symbol: &str,
    new_body: &str,
) -> Result<String> {
    let body = new_body.trim();
    if !body.starts_with('(') {
        return Err(Error::BodyNotSexp);
    }
    let (start, end) = plugin.node_bounds(content, symbol)?;
    let updated = replace_region(content, start, end, body);
    plugin.check_file(&updated).map_err(|e| match e {
        Error::Syntax(detail) => Error::SyntaxAfterEdit {
            operation: "replace".into(),
            detail,
        },
        other => other,
    })?;
    Ok(updated)
}

pub fn insert_after(
    plugin: &dyn LanguagePlugin,
    content: &str,
    after_symbol: &str,
    node: &str,
) -> Result<String> {
    let body = node.trim();
    if body.is_empty() {
        return Err(Error::EmptyForm);
    }
    if !body.starts_with('(') {
        return Err(Error::BodyNotSexp);
    }

    let pos = find_insert_position(plugin, content, after_symbol)?;
    let blank = content_blank(content);
    let insertion = if pos == 0 && blank {
        body.to_string()
    } else {
        format!("\n\n{body}")
    };
    let updated = replace_region(content, pos, pos, &insertion);
    plugin.check_file(&updated).map_err(|e| match e {
        Error::Syntax(detail) => Error::SyntaxAfterEdit {
            operation: "insert".into(),
            detail,
        },
        other => other,
    })?;
    Ok(updated)
}

fn find_insert_position(
    plugin: &dyn LanguagePlugin,
    content: &str,
    after_symbol: &str,
) -> Result<usize> {
    if is_anchor_start(after_symbol) {
        if content_blank(content) {
            return Ok(0);
        }
        return Err(Error::StartAnchorOnNonempty(
            ANCHOR_START.into(),
            ANCHOR_END.into(),
        ));
    }
    if is_anchor_end(after_symbol) {
        return end_of_forms(plugin, content);
    }
    let (_, end) = plugin.node_bounds(content, after_symbol)?;
    Ok(end)
}

fn end_of_forms(plugin: &dyn LanguagePlugin, content: &str) -> Result<usize> {
    if content_blank(content) {
        return Ok(0);
    }
    plugin
        .top_level_forms(content)?
        .last()
        .map(|f| f.end)
        .ok_or_else(|| Error::Message("No forms".into()))
}
