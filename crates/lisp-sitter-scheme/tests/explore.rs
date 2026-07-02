#![allow(dead_code)]

use tree_sitter::{Node, Parser};

fn dump(node: Node, src: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    let preview: String = node
        .utf8_text(src.as_bytes())
        .unwrap_or("")
        .chars()
        .take(50)
        .collect();
    eprintln!(
        "{indent}{} [{}..{}] {preview:?}",
        node.kind(),
        node.start_byte(),
        node.end_byte()
    );
    if depth < 5 {
        let mut c = node.walk();
        for child in node.children(&mut c) {
            dump(child, src, depth + 1);
        }
    }
}

#[test]
#[ignore]
fn explore_scheme_library() {
    let code = "(define-library (my lib)\n  (export foo)\n  (import (scheme base)))\n";
    let mut p = Parser::new();
    p.set_language(&tree_sitter_scheme::LANGUAGE.into())
        .unwrap();
    let tree = p.parse(code, None).unwrap();
    dump(tree.root_node(), code, 0);
}
