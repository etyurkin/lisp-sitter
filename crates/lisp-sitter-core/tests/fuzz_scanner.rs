//! Property tests: the byte scanner and edit primitives must never panic on
//! arbitrary (including adversarial / multibyte) input. These guard the class
//! of char-boundary and char-literal bugs found during the audit.

use lisp_sitter_core::scan::{replace_region, scan_parens_in};
use lisp_sitter_core::sexp_reader::{complete_form_in, skip_sexp_in, Dialect};
use proptest::prelude::*;

proptest! {
    #[test]
    fn scan_parens_never_panics(s in ".*") {
        let _ = scan_parens_in(&s, Dialect::Generic);
        let _ = scan_parens_in(&s, Dialect::Elisp);
    }

    #[test]
    fn skip_sexp_never_panics(s in ".*", start in 0usize..2048) {
        let b = s.as_bytes();
        // start may be past the end or inside a multibyte char — must not panic.
        let _ = skip_sexp_in(b, start, Dialect::Generic);
        let _ = skip_sexp_in(b, start, Dialect::Elisp);
    }

    #[test]
    fn replace_region_never_panics(s in ".*", start in 0usize..2048, end in 0usize..2048, repl in ".*") {
        // start/end may land inside multibyte chars or out of bounds.
        let out = replace_region(&s, start, end, &repl);
        prop_assert!(repl.is_empty() || out.contains(&repl));
    }

    #[test]
    fn complete_form_never_panics(s in ".*") {
        let _ = complete_form_in(&s, Dialect::Generic);
        let _ = complete_form_in(&s, Dialect::Elisp);
    }

    #[test]
    fn complete_balanced_stays_balanced(s in ".*") {
        // Completing already-valid input keeps it valid (no stray parens added).
        if scan_parens_in(&s, Dialect::Generic).is_none() {
            if let Some(out) = complete_form_in(&s, Dialect::Generic) {
                prop_assert!(scan_parens_in(&out, Dialect::Generic).is_none());
            }
        }
    }
}

// Targeted regressions for the exact char-boundary panic the audit found.
#[test]
fn replace_region_mid_multibyte_does_not_panic() {
    // Offsets 1 and 2 land inside the 3-byte '日'.
    let out = replace_region("日本語", 1, 2, "X");
    assert!(out.contains('X'));
}

#[test]
fn replace_region_out_of_bounds_offsets() {
    let out = replace_region("abc", 99, 200, "Z");
    assert_eq!(out, "abcZ");
}
