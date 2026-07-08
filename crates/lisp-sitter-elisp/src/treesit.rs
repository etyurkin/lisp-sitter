use lisp_sitter_core::definers::Definer;

/// Base top-level definition forms recognized for Emacs Lisp (before any
/// user-configured `extra_definers` are appended).
pub fn base_definers() -> Vec<Definer> {
    [
        "defun",
        "defsubst",
        "defmacro",
        "cl-defun",
        "cl-defmacro",
        "cl-defsubst",
        "cl-defmethod",
        "cl-defgeneric",
        "cl-defstruct",
        "defvar",
        "defvar-local",
        "defconst",
        "defcustom",
        "defface",
        "defgroup",
        "define-minor-mode",
        "define-derived-mode",
        "define-globalized-minor-mode",
        "define-error",
        "defalias",
        "ert-deftest",
        "define-advice",
    ]
    .into_iter()
    .map(Definer::second)
    .collect()
}

#[cfg(test)]
mod tests {
    use crate::ElispPlugin;
    use lisp_sitter_core::LanguagePlugin;

    #[test]
    fn parses_defun_outline() {
        let content = "(defun a () 1)\n(defvar b 2)\n";
        let forms = ElispPlugin::new().top_level_forms(content).unwrap();
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "defun:a");
        assert_eq!(forms[1].label, "defvar:b");
    }

    #[test]
    fn recognizes_broad_forms() {
        let content = "(cl-defun f () 1)\n(defcustom o t \"doc\")\n\
                       (define-minor-mode m \"doc\")\n(cl-defmethod g ((x integer)) x)\n\
                       (ert-deftest tst () (should t))\n(defalias 'al 'f)\n";
        let forms = ElispPlugin::new().top_level_forms(content).unwrap();
        let labels: Vec<&str> = forms.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"cl-defun:f"), "{labels:?}");
        assert!(labels.contains(&"defcustom:o"), "{labels:?}");
        assert!(labels.contains(&"define-minor-mode:m"), "{labels:?}");
        assert!(labels.contains(&"cl-defmethod:g"), "{labels:?}");
        assert!(labels.contains(&"ert-deftest:tst"), "{labels:?}");
        assert!(labels.contains(&"defalias:al"), "{labels:?}");
    }

    #[test]
    fn extra_definer_recognized() {
        let plugin = ElispPlugin::with_extra_definers(&["define-widget".to_string()]);
        let forms = plugin
            .top_level_forms("(define-widget my-w 'item \"doc\")\n")
            .unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].label, "define-widget:my-w");
    }

    #[test]
    fn bounds_for_minor_mode() {
        let content = "(defun a () 1)\n(define-minor-mode my-mode \"doc\")\n";
        let b = ElispPlugin::new().node_bounds(content, "my-mode").unwrap();
        assert_eq!(&content[b.0..b.1], "(define-minor-mode my-mode \"doc\")");
    }
}
