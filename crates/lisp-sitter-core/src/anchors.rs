pub const ANCHOR_START: &str = "__start__";
pub const ANCHOR_END: &str = "__end__";

pub fn is_anchor_start(symbol: &str) -> bool {
    let s = symbol.trim();
    s.is_empty() || s == ANCHOR_START
}

pub fn is_anchor_end(symbol: &str) -> bool {
    symbol.trim() == ANCHOR_END
}
