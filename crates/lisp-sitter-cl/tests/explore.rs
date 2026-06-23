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
fn explore_cl() {
    let code = "(defun alpha () 1)\n(defmacro beta () '(+ 1 2))\n";
    let mut p = Parser::new();
    p.set_language(&tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into())
        .unwrap();
    let tree = p.parse(code, None).unwrap();
    dump(tree.root_node(), code, 0);
}

#[test]
#[ignore]
fn defun_name_field() {
    let code = "(defun alpha () 1)\n";
    let mut p = Parser::new();
    p.set_language(&tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into())
        .unwrap();
    let tree = p.parse(code, None).unwrap();
    let root = tree.root_node();
    let mut c = root.walk();
    for child in root.children(&mut c) {
        let mut c2 = child.walk();
        for inner in child.children(&mut c2) {
            if inner.kind() == "defun" {
                eprintln!(
                    "name field: {:?}",
                    inner
                        .child_by_field_name("name")
                        .map(|n| n.utf8_text(code.as_bytes()).unwrap().to_string())
                );
            }
        }
    }
}

#[test]
#[ignore]
fn explore_cl_defclass() {
    let code = "(defclass foo () ((slot :initform 0)))\n";
    let mut p = Parser::new();
    p.set_language(&tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into())
        .unwrap();
    let tree = p.parse(code, None).unwrap();
    dump(tree.root_node(), code, 0);
}
