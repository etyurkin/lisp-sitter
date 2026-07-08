use lisp_sitter_core::definers::Definer;

/// Base top-level definition forms recognized for Common Lisp.
pub fn base_definers() -> Vec<Definer> {
    [
        "defun",
        "defmacro",
        "defclass",
        "defgeneric",
        "defmethod",
        "defvar",
        "defparameter",
        "defconstant",
        "defstruct",
        "deftype",
        "define-condition",
        "defpackage",
        "in-package",
        "defsetf",
        "define-compiler-macro",
        "define-symbol-macro",
    ]
    .into_iter()
    .map(Definer::second)
    .collect()
}

#[cfg(test)]
mod tests {
    use crate::CommonLispPlugin;
    use lisp_sitter_core::LanguagePlugin;

    #[test]
    fn parses_defun_and_defmacro() {
        let content = "(defun alpha () 1)\n(defmacro beta () '(+ 1 2))\n";
        let forms = CommonLispPlugin::new().top_level_forms(content).unwrap();
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].label, "defun:alpha");
        assert_eq!(forms[1].label, "defmacro:beta");
    }

    #[test]
    fn parses_defclass_list_lit() {
        let content = "(defclass foo () ((slot :initform 0)))\n";
        let forms = CommonLispPlugin::new().top_level_forms(content).unwrap();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].name.as_deref(), Some("foo"));
        assert!(forms[0].label.starts_with("defclass:"));
    }

    #[test]
    fn recognizes_broad_forms() {
        let content = "(defvar *x* 1)\n(defparameter *y* 2)\n(defpackage :my-pkg (:use :cl))\n\
                       (in-package :my-pkg)\n(define-condition my-err (error) ())\n(defstruct point x y)\n";
        let forms = CommonLispPlugin::new().top_level_forms(content).unwrap();
        let labels: Vec<&str> = forms.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"defvar:*x*"), "{labels:?}");
        assert!(labels.contains(&"defparameter:*y*"), "{labels:?}");
        assert!(labels.contains(&"defpackage:my-pkg"), "{labels:?}");
        assert!(labels.contains(&"in-package:my-pkg"), "{labels:?}");
        assert!(labels.contains(&"define-condition:my-err"), "{labels:?}");
        assert!(labels.contains(&"defstruct:point"), "{labels:?}");
    }

    #[test]
    fn defmethod_qualifier_name() {
        let content = "(defmethod foo :around ((x integer)) x)\n";
        let forms = CommonLispPlugin::new().top_level_forms(content).unwrap();
        assert_eq!(forms[0].name.as_deref(), Some("foo"));
    }
}
