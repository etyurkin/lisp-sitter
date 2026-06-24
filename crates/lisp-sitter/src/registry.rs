use lisp_sitter_cl::CommonLispPlugin;
use lisp_sitter_core::Registry;
use lisp_sitter_elisp::ElispPlugin;
use lisp_sitter_scheme::SchemePlugin;

pub fn default_registry() -> Registry {
    let cfg = crate::config::Config::load();
    let mut reg = Registry::new();
    reg.register(Box::new(ElispPlugin::with_extra_definers(cfg.definers_for("elisp"))));
    reg.register(Box::new(CommonLispPlugin::with_extra_definers(cfg.definers_for("commonlisp"))));
    reg.register(Box::new(SchemePlugin::with_extra_definers(cfg.definers_for("scheme"))));
    cfg.apply(&mut reg);
    reg
}
