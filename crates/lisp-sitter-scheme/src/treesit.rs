use lisp_sitter_core::definers::{Definer, NameStrategy};

/// Base top-level definition forms recognized for Scheme.
pub fn base_definers() -> Vec<Definer> {
    let mut defs: Vec<Definer> = [
        "define",
        "define-syntax",
        "define-record-type",
        "define-values",
        "define-structure",
    ]
    .into_iter()
    .map(Definer::second)
    .collect();
    defs.push(Definer::new("define-library", NameStrategy::LibraryList));
    defs
}

#[cfg(test)]
mod tests {
    use crate::SchemePlugin;
    use lisp_sitter_core::LanguagePlugin;

    #[test]
    fn parses_define_forms() {
        let content = "(define foo 1)\n(define (bar x) (+ x 1))\n";
        let forms = SchemePlugin::new().top_level_forms(content).unwrap();
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "define:foo");
        assert_eq!(forms[1].label, "define:bar");
    }

    #[test]
    fn parses_define_library() {
        let content = "(define-library (my lib)\n  (export foo))\n";
        let forms = SchemePlugin::new().top_level_forms(content).unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("my lib"));
    }

    #[test]
    fn parses_define_record_type() {
        let content =
            "(define-record-type point (make-point x y) point? (x point-x) (y point-y))\n";
        let forms = SchemePlugin::new().top_level_forms(content).unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("point"));
    }
}
