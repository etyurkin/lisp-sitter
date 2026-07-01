mod plugin;
pub mod treesit;

pub use plugin::SchemePlugin;

use lisp_sitter_core::DefinerSet;

pub fn definer_set() -> DefinerSet {
    DefinerSet::new(treesit::base_definers())
}
