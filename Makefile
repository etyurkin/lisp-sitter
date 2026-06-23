CARGO ?= cargo
BIN_CRATE := crates/lisp-sitter
RELEASE_BIN := target/release/lisp-sitter
DEBUG_BIN := target/debug/lisp-sitter

.PHONY: all build release test install clean check fmt lint help run mcp-install completions doc

all: release

build:; $(CARGO) build

release:; $(CARGO) build --release

test:; $(CARGO) test --workspace

install: release; $(CARGO) install --path $(BIN_CRATE)

clean:; $(CARGO) clean

check:; $(CARGO) check --workspace

fmt:; $(CARGO) fmt --all

lint:; $(CARGO) clippy --workspace --all-targets -- -D warnings

run: release; $(RELEASE_BIN) --help

mcp-install: release; $(RELEASE_BIN) mcp install

completions: release
	$(RELEASE_BIN) completions bash > /tmp/lisp-sitter.bash
	$(RELEASE_BIN) completions zsh > /tmp/lisp-sitter.zsh
	$(RELEASE_BIN) completions fish > /tmp/lisp-sitter.fish
	@echo "Completion scripts written to /tmp/lisp-sitter.{bash,zsh,fish}"

doc:
	@echo "docs/usage.md — lisp-sitter usage guide"
	@echo "Run 'make doc-view' to view (requires pandoc or similar)"

doc-view:
	@if command -v pandoc >/dev/null 2>&1; then pandoc docs/usage.md -s -o /tmp/lisp-sitter-usage.html && open /tmp/lisp-sitter-usage.html; else echo "Install pandoc to view docs as HTML"; fi

help:
	@echo "lisp-sitter — structural editing for Lisp dialects"
	@echo ""
	@echo "Build:"
	@echo "  all      build release binary (default)"
	@echo "  build    debug build"
	@echo "  release  optimized build -> $(RELEASE_BIN)"
	@echo "  clean    remove target/"
	@echo ""
	@echo "Test & check:"
	@echo "  test     run workspace tests"
	@echo "  check    cargo check"
	@echo "  lint     clippy with warnings denied"
	@echo "  fmt      rustfmt all crates"
	@echo ""
	@echo "Install:"
	@echo "  install       install lisp-sitter to \$$HOME/.cargo/bin"
	@echo "  mcp-install   register MCP server in ~/.cursor/mcp.json"
	@echo ""
	@echo "Shell completions:"
	@echo "  completions   generate bash/zsh/fish completion scripts"
	@echo "  Usage: eval \"\$$(lisp-sitter completions bash)\""
	@echo ""
	@echo "Documentation:"
	@echo "  doc       show docs location"
	@echo "  doc-view  open docs as HTML (requires pandoc)"
	@echo ""
	@echo "Other:"
	@echo "  run      show CLI help"
	@echo "  help     this message"
