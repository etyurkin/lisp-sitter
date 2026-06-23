use lisp_sitter_cl::CommonLispPlugin;
use lisp_sitter_core::Registry;
use lisp_sitter_elisp::ElispPlugin;
use lisp_sitter_scheme::SchemePlugin;

pub fn default_registry() -> Registry {
    let mut reg = Registry::new();
    reg.register(Box::new(ElispPlugin));
    reg.register(Box::new(CommonLispPlugin));
    reg.register(Box::new(SchemePlugin));
    let cfg = crate::config::Config::load();
    cfg.apply(&mut reg);
    reg
}
