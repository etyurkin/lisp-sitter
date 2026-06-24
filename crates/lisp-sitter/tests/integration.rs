//! Integration tests: run lisp-sitter CLI operations against realistic files
//! that mix the full range of top-level definition forms, char literals,
//! Unicode, and structural refactoring operations.
//!
//! These tests call into the ops/transform modules directly (not via the
//! executable) so they verify the same code paths as the CLI and MCP server.

use lisp_sitter::default_registry;
use lisp_sitter_core::edit::replace_node;
use lisp_sitter::ops;
use lisp_sitter::transform;

// ── helpers ──────────────────────────────────────────────────────────

fn reg() -> lisp_sitter_core::Registry {
    default_registry()
}

fn tmp_file(prefix: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("lisp-sitter-integ-{}-{}", std::process::id(), prefix));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("test.el");
    std::fs::write(&p, body).unwrap();
    (d, p)
}

fn tmp_cl(prefix: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("lisp-sitter-integ-{}-{}", std::process::id(), prefix));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("test.lisp");
    std::fs::write(&p, body).unwrap();
    (d, p)
}

fn tmp_scm(prefix: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("lisp-sitter-integ-{}-{}", std::process::id(), prefix));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("test.scm");
    std::fs::write(&p, body).unwrap();
    (d, p)
}

// ── Realistic elisp file (mixing defun, defcustom, minor-mode, etc.) ────

const ELISP_REALISTIC: &str = r#";;; test.el --- integration test -*- lexical-binding: t -*-
(require 'cl-lib)

(defgroup test nil "Test group." :group 'tools)

(defcustom test-option t
  "An option."
  :type 'boolean)

(defvar-local test-state nil
  "Buffer-local state.")

(defun test-fn (x)
  "Double X."
  (* 2 x))

(cl-defun test-cl-fn (&key a b)
  "CL function."
  (+ a b))

(cl-defmethod test-method ((x integer))
  "A method."
  (1+ x))

(define-minor-mode test-mode
  "A minor mode."
  :lighter " Test")

(define-derived-mode test-major-mode prog-mode "Test"
  "A major mode.")

(ert-deftest test-test ()
  "A test."
  (should (= 4 (test-fn 2))))

(provide 'test)
;;; test.el ends here
"#;

#[test]
fn elisp_realistic_works() {
    let reg = reg();
    let (dir, path) = tmp_file("elisp_realistic", ELISP_REALISTIC);

    // ── tree: all definitions visible ───────────────────────────
    let outline = ops::tree(&reg, path.to_str().unwrap()).unwrap();
    for def in ["test-fn", "test-cl-fn", "test-method", "test-option", "test-state", "test-mode", "test-major-mode", "test-test"] {
        assert!(outline.contains(def), "missing from outline: {def}\n{outline}");
    }

    // ── bounds: every named form addressable ────────────────────
    assert!(ops::bounds(&reg, path.to_str().unwrap(), "test-fn").is_ok());
    assert!(ops::bounds(&reg, path.to_str().unwrap(), "test-option").is_ok());
    assert!(ops::bounds(&reg, path.to_str().unwrap(), "test-mode").is_ok());
    assert!(ops::bounds(&reg, path.to_str().unwrap(), "test-method").is_ok());
    assert!(ops::bounds(&reg, path.to_str().unwrap(), "test-test").is_ok());

    // ── get: form text is complete ──────────────────────────────
    let text = ops::get_form(&reg, path.to_str().unwrap(), "test-fn").unwrap();
    assert!(text.contains("defun test-fn"), "get returned wrong text: {text}");
    assert!(text.contains("* 2 x"), "get missing body: {text}");

    // ── replace: round-trip cleanly ─────────────────────────────
    let new = "(defun test-fn (x)\n  \"Double X.\"\n  (* 3 x))\n";
    let updated = replace_node(reg.plugin_for_path(path.to_str().unwrap()).unwrap(), ELISP_REALISTIC, "test-fn", new).unwrap();
    assert!(updated.contains("* 3 x"));
    assert!(reg.plugin_for_path(path.to_str().unwrap()).unwrap().check_file(&updated).is_ok());

    // ── check: file is valid ────────────────────────────────────
    assert_eq!(ops::check_structural_file(&reg, path.to_str().unwrap()).unwrap(), "OK");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elisp_fmt_unicode() {
    let reg = reg();
    let (dir, path) = tmp_file("fmt_uni", "(defun f ()\n  \"héllo 日本語\")\n");
    let fmt = ops::format_file(&reg, path.to_str().unwrap()).unwrap();
    assert!(fmt.contains("héllo"), "unicode corrupted by fmt: {fmt}");
    assert!(fmt.contains("日本語"), "unicode corrupted by fmt: {fmt}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elisp_char_literals() {
    let reg = reg();
    let (dir, path) = tmp_file("char", "(setq c ?\\( d ?\\) e ?\\\\)\n");
    assert_eq!(ops::check_structural_file(&reg, path.to_str().unwrap()).unwrap(), "OK",
        "char literals with parens should be valid");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elisp_extract_works() {
    let reg = reg();
    let content = "(defun area (r)\n  (* 3.14 (* r r)))\n";
    let (dir, path) = tmp_file("extract", content);
    let result = transform::extract(&reg, path.to_str().unwrap(), "area", "(* r r)", "square", &["r"]);
    assert!(result.is_ok(), "extract failed: {:?}", result.err());
    let got = result.unwrap();
    assert!(got.contains("defun square"), "should define square: {got}");
    assert!(got.contains("(square r)"), "should call square: {got}");
    assert!(reg.plugin_for_path(path.to_str().unwrap()).unwrap().check_file(&got).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elisp_rename_string_safe() {
    let reg = reg();
    let content = "(defun foo () 1)\n(defun bar () (message \"call (foo) here\"))\n";
    let (dir, path) = tmp_file("rename_safe", content);
    let result = transform::rename(&reg, path.to_str().unwrap(), "foo", "baz", transform::RefsMode::HeadOnly);
    assert!(result.is_ok());
    let got = result.unwrap();
    assert!(got.contains("\"call (foo) here\""), "string must be untouched: {got}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elisp_flatten_arg_substitution() {
    let reg = reg();
    let content = "(defun add1 (x)\n  (+ x 1))\n\n(defun foo ()\n  (add1 2))\n";
    let (dir, path) = tmp_file("flatten_subst", content);
    let result = transform::flatten(&reg, path.to_str().unwrap(), "add1");
    assert!(result.is_ok(), "flatten failed: {:?}", result.err());
    let got = result.unwrap();
    assert!(!got.contains("(defun add1"), "definition should be removed: {got}");
    assert!(got.contains("(+ 2 1)"), "should inline with arg substitution: {got}");
    assert!(!got.contains("(add1 2)"), "call should not remain: {got}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── CL integration ───────────────────────────────────────────────────

const CL_REALISTIC: &str = r#"(defpackage :my-pkg
  (:use :cl)
  (:export #:add #:*x*))
(in-package :my-pkg)

(defun add (a b)
  "Add A and B."
  (+ a b))

(defvar *x* 42
  "The answer.")

(defparameter *name* "hello"
  "A name.")

(defstruct point
  "A 2D point."
  x y)
"#;

#[test]
fn cl_realistic_works() {
    let reg = reg();
    let (dir, path) = tmp_cl("cl_real", CL_REALISTIC);

    let outline = ops::tree(&reg, path.to_str().unwrap()).unwrap();
    for def in ["add", "*x*", "*name*", "point"] {
        assert!(outline.contains(def), "missing from CL outline: {def}\n{outline}");
    }

    assert!(ops::bounds(&reg, path.to_str().unwrap(), "add").is_ok());
    assert_eq!(ops::check_structural_file(&reg, path.to_str().unwrap()).unwrap(), "OK");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Scheme integration ────────────────────────────────────────────────

const SCHEME_REALISTIC: &str = r#"(define-library (my lib)
  (export x greet)
  (define x 1)
  (define (greet n)
    "Greet N."
    (+ n "!")))
"#;

#[test]
fn scheme_realistic_works() {
    let reg = reg();
    let (dir, path) = tmp_scm("scheme_real", SCHEME_REALISTIC);

    let outline = ops::tree(&reg, path.to_str().unwrap()).unwrap();
    assert!(outline.contains("define-library"), "missing library form: {outline}");
    assert!(outline.contains("my lib"), "missing library name: {outline}");

    assert!(ops::bounds(&reg, path.to_str().unwrap(), "my lib").is_ok(),
        "should find library by name");
    assert_eq!(ops::check_structural_file(&reg, path.to_str().unwrap()).unwrap(), "OK");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Rename quoted refs ──────────────────────────────────────────────

#[test]
fn elisp_rename_sharp_quote_refs() {
    let reg = reg();
    let content = "(defun foo () 1)\n(add-hook 'h #'foo)\n";
    let (dir, path) = tmp_file("rename_sharp", content);
    let result = transform::rename(&reg, path.to_str().unwrap(), "foo", "baz", transform::RefsMode::HeadAndSharp);
    assert!(result.is_ok());
    let got = result.unwrap();
    assert!(got.contains("#'baz"), "#' should be renamed: {got}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elisp_rename_in_string_not_touched() {
    let reg = reg();
    let content = "(defun foo () 1)\n(let ((s \"call (foo) here\")) (foo))\n";
    let (dir, path) = tmp_file("rename_str", content);
    let result = transform::rename(&reg, path.to_str().unwrap(), "foo", "bar", transform::RefsMode::HeadAndSharp);
    assert!(result.is_ok());
    let got = result.unwrap();
    assert!(got.contains("\"call (foo) here\""), "string should be untouched: {got}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Format / indent preservation ─────────────────────────────────────

#[test]
fn fmt_multiline_preserves_structure() {
    let reg = reg();
    let content = "(defun foo (x)\n  (+ x 1))\n";
    let (dir, path) = tmp_file("fmt_multi", content);
    assert_eq!(ops::format_file(&reg, path.to_str().unwrap()).unwrap(), content);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_fixes_indentation() {
    let reg = reg();
    let content = "(defun foo (x)\n      (+ x 1))\n";
    let expected = "(defun foo (x)\n  (+ x 1))\n";
    let (dir, path) = tmp_file("fmt_fix", content);
    assert_eq!(ops::format_file(&reg, path.to_str().unwrap()).unwrap(), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_nested_fixes_two_levels() {
    let reg = reg();
    let content = "(defun foo (x)\n  (let ((y 1))\n  y))\n";
    let expected = "(defun foo (x)\n  (let ((y 1))\n    y))\n";
    let (dir, path) = tmp_file("fmt_nest", content);
    assert_eq!(ops::format_file(&reg, path.to_str().unwrap()).unwrap(), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cl_char_literals() {
    let reg = reg();
    let content = "(defun f () #\\()\n";
    let (dir, path) = tmp_cl("cl_char", content);
    assert_eq!(ops::check_structural_file(&reg, path.to_str().unwrap()).unwrap(), "OK");
    let _ = std::fs::remove_dir_all(&dir);
}
