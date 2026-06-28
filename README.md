# lisp-sitter

Structural editing CLI for Emacs Lisp, Scheme, and Common Lisp — powered by [tree-sitter](https://tree-sitter.github.io/).

Coding agents (Claude CLI, Cursor CLI, etc.) usually edit files with line-based search/replace. That breaks easily on Lisp code where structure matters. **lisp-sitter** operates on whole top-level forms: find a `defun`/`define` by name, replace or insert complete s-expressions, validate before write. All structural operations are also exposed as **MCP tools** for agent-friendly access.

## Supported languages

| Language | Extensions | Top-level forms |
|----------|------------|-----------------|
| Emacs Lisp | `.el` | `defun`, `defmacro`, `defsubst`, `cl-defun`, `defvar`, `defconst`, `defcustom` |
| Common Lisp | `.lisp`, `.cl` | `defun`, `defmacro`, `defclass`, `defgeneric`, `defmethod` |
| Scheme | `.scm`, `.ss`, `.sld` | `define`, `define-syntax`, `define-library` |

Language is inferred from the file extension.

## Install

Requires Rust 1.70+ and a C compiler (for tree-sitter grammars).

```bash
git clone <repo-url> lisp-sitter && cd lisp-sitter
make install          # builds release, installs to ~/.cargo/bin
```

Or without Make:

```bash
cargo install --path crates/lisp-sitter
```

## How it works

- **Parse** — tree-sitter grammars ([elisp](https://github.com/Wilfred/tree-sitter-elisp), [commonlisp](https://github.com/theHamsta/tree-sitter-commonlisp), [scheme](https://github.com/6cdh/tree-sitter-scheme))
- **Navigate** — top-level form names and byte ranges
- **Edit** — splice replacement text, re-parse to validate
- **Fallback** — s-expression scanner when the parse tree is incomplete

Tree-sitter sees syntax, not semantics: `(foo (let ...))` is a list, not "macro vs function call". That is sufficient for structural replace/insert.

## Development

```bash
make help     # list targets
make test     # run all tests
make release  # build target/release/lisp-sitter
make lint     # clippy
make fmt      # rustfmt
```

### Workspace layout

```
crates/
  lisp-sitter-core/    # plugin trait, edit engine, sexp fallback
  lisp-sitter-elisp/
  lisp-sitter-cl/
  lisp-sitter-scheme/
  lisp-sitter/         # CLI binary + MCP server
```

## License

MIT — see [LICENSE](LICENSE).
