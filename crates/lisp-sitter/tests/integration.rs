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

#[test]
fn malformed_source_refuses_destructive_edit() {
    let reg = reg();
    let content = "(defun a () 1)\n\n(defun b (x\n  (+ x 1))\n\n(defun c () 3)\n";
    let (dir, path) = tmp_file("malformed_guard", content);
    let p = reg.plugin_for_path(path.to_str().unwrap()).unwrap();
    let read = || std::fs::read_to_string(&path).unwrap();

    let replace = replace_node(p, &read(), "b", "(defun b (x) (+ x 1))");
    assert!(matches!(replace, Err(lisp_sitter_core::Error::MalformedSource(_))), "{replace:?}");

    let remove = transform::remove_form(&reg, path.to_str().unwrap(), "b", true);
    assert!(matches!(remove, Err(lisp_sitter_core::Error::MalformedSource(_))), "{remove:?}");

    assert!(read().contains("(defun c () 3)"), "form c must not be deleted");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stdin_path_outputs_only_result() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let bin = env!("CARGO_BIN_EXE_lisp-sitter");
    let body = "(defun foo () 1)";

    let run = |args: &[&str]| -> String {
        let mut child = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn lisp-sitter");
        child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    assert_eq!(run(&["--lang", "elisp", "bounds", "-", "foo"]), "0:16");
    assert_eq!(run(&["--lang", "elisp", "check", "-"]), "-: OK");
    assert_eq!(run(&["check-node", "--lang", "elisp", "--body-file", "-"]), "OK");
}

// ── cross-file rename ─────────────────────────────────────────────────

#[test]
fn rename_project_updates_definition_and_all_callers() {
    let reg = reg();
    let dir = std::env::temp_dir().join(format!("lisp-sitter-integ-{}-rename-proj", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.el");
    let b = dir.join("b.el");
    std::fs::write(&a, "(defun helper (x)\n  (* x x))\n\n(defun main ()\n  (helper 3))\n").unwrap();
    std::fs::write(&b, "(defun other ()\n  (helper 5))\n").unwrap();

    let paths = ops::expand_paths(dir.to_str().unwrap());
    let changed = transform::rename_project(&reg, &paths, "helper", "square", transform::RefsMode::HeadAndSharp).unwrap();
    assert_eq!(changed.len(), 2, "both files should change");
    for (p, c) in &changed {
        ops::atomic_write(p, c).unwrap();
    }

    let a_out = std::fs::read_to_string(&a).unwrap();
    assert!(a_out.contains("(defun square (x)"), "definition renamed: {a_out}");
    assert!(a_out.contains("(square 3)"), "same-file caller renamed: {a_out}");
    assert!(!a_out.contains("helper"), "no stale references: {a_out}");

    let b_out = std::fs::read_to_string(&b).unwrap();
    assert!(b_out.contains("(square 5)"), "cross-file caller renamed: {b_out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rename_project_errors_when_symbol_undefined() {
    let reg = reg();
    let dir = std::env::temp_dir().join(format!("lisp-sitter-integ-{}-rename-missing", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.el"), "(defun foo () 1)\n").unwrap();

    let paths = ops::expand_paths(dir.to_str().unwrap());
    let r = transform::rename_project(&reg, &paths, "nonexistent", "bar", transform::RefsMode::HeadAndSharp);
    assert!(matches!(r, Err(lisp_sitter_core::Error::FormNotFound(_))), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── project analysis ──────────────────────────────────────────────────

#[test]
fn analyze_reports_unused_arity_and_unresolved() {
    let reg = reg();
    let dir = std::env::temp_dir().join(format!("lisp-sitter-integ-{}-analyze", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a.el");
    std::fs::write(&p, "(defun add (a b) (+ a b))\n(defun lonely () 1)\n(defun go () (add 1) (mystery 2))\n").unwrap();

    let paths = ops::expand_paths(dir.to_str().unwrap());
    let report = lisp_sitter::analyze::analyze(&reg, &paths, lisp_sitter::analyze::Options::all()).unwrap();
    assert!(report.contains("unused") && report.contains("lonely"), "{report}");
    assert!(report.contains("arity") && report.contains("`add`"), "{report}");
    assert!(report.contains("unresolved") && report.contains("mystery"), "{report}");
    // a builtin call must not be reported as unresolved
    assert!(!report.contains("`+`"), "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn analyze_suppresses_autoloaded_and_required() {
    let reg = reg();
    let dir = std::env::temp_dir().join(format!(
        "lisp-sitter-integ-{}-analyze2",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // autoloaded form should NOT appear as unused even with zero internal callers
    let p = dir.join("pub.el");
    std::fs::write(
        &p,
        ";;;###autoload\n(defun my-public-api () 1)\n\
         (defun my-internal () (my-public-api))\n",
    )
    .unwrap();

    let paths = ops::expand_paths(dir.to_str().unwrap());
    let report =
        lisp_sitter::analyze::analyze(&reg, &paths, lisp_sitter::analyze::Options::all())
            .unwrap();
    assert!(
        !report.contains("my-public-api"),
        "autoloaded form should be suppressed: {report}"
    );

    // symbol with a prefix from a required package must NOT appear as unresolved
    let p2 = dir.join("consumer.el");
    std::fs::write(
        &p2,
        "(require 'dash)\n\
         (defun my-consumer () (dash-map (lambda (x) x) '(1 2 3)))\n",
    )
    .unwrap();

    let paths2 = ops::expand_paths(dir.to_str().unwrap());
    let report2 =
        lisp_sitter::analyze::analyze(&reg, &paths2, lisp_sitter::analyze::Options::all())
            .unwrap();
    assert!(
        !report2.contains("dash-map"),
        "required-prefix symbol should be suppressed: {report2}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
