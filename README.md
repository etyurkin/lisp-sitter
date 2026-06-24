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

## Quick start

```bash
# List top-level forms (line:column suffix on each label)
lisp-sitter tree src/foo.el

# Byte range of a named form
lisp-sitter bounds src/foo.el my-function

# Print the full text of a form
lisp-sitter get src/foo.el my-function

# Replace a form (stdout); add --write to save
lisp-sitter replace src/foo.el my-function \
  --body '(defun my-function () 42)' --write

# Insert after a symbol, at file start, or at end
lisp-sitter insert src/foo.el my-function \
  --node '(defun helper () t)' --write
lisp-sitter insert new.scm __start__ --node '(define version 1)' --write
lisp-sitter insert lib.lisp __end__ --node '(defun tail () nil)' --write

# Complete missing parens
lisp-sitter complete --lang scheme --body '(define (fib n) (if (< n 2)'
# → (define (fib n) (if (< n 2) n ...))

# Re-indent a file
lisp-sitter fmt src/foo.el --write

# Validate
lisp-sitter check src/foo.el
lisp-sitter check-node --lang scheme --body '(define x 1)'
```

### Anchors

| Anchor | Meaning |
|--------|---------|
| `__start__` | Insert as the first form (empty file only) |
| `__end__` | Append after the last top-level form |
| *symbol* | Insert immediately after the named form |

### stdin

`--body-file -` and `--node-file -` read from stdin:

```bash
echo '(defun x () 1)' | lisp-sitter replace foo.el old --body-file - --write
```

## Commands

| Command | Description |
|---------|-------------|
| `tree PATH` | Outline of top-level forms, one per line (`defun:foo@12:1`) |
| `bounds PATH SYMBOL` | Byte positions `START:END` for a named form |
| `get PATH SYMBOL` | Print the full text of a named top-level form |
| `replace PATH SYMBOL` | Replace a form; requires `--body` or `--body-file` |
| `insert PATH AFTER` | Insert a form; requires `--node` or `--node-file` |
| `complete` | Append missing `)` to an unbalanced s-expression |
| `fmt PATH` | Re-indent a file (depth-based, 2-space indent). `--write` to save |
| `eval PATH` | Run dialect-specific validation (byte-compile, sbcl, guile…) |
| `remove PATH SYMBOL` | Remove a form; optionally replace call sites with ignore |
| `move PATH SYMBOL --after ANCHOR` | Reorder a form after another symbol, `__start__`, or `__end__` |
| `substitute PATH SYMBOL` | Replace a sub-expression inside a form using `--pattern` / `--replacement` |
| `extract PATH SYMBOL` | Extract a sub-expression into a new function |
| `rename PATH OLD NEW` | Rename a form and its call sites |
| `wrap PATH SYMBOL` | Wrap body in `progn`, `let`, or `if` |
| `check PATH` | Validate file → `OK` or syntax error on stderr |
| `check PATH --semantic` | Deep validation (docstrings, missing provide) |
| `check-node` | Validate one form; `--lang elisp\|commonlisp\|scheme` |
| `mcp serve` | Run MCP server on stdio |
| `mcp install` | Add server to `~/.cursor/mcp.json` (or `--claude`) |

`replace`, `insert`, `fmt`, `remove`, `move`, `substitute`, `extract`, `rename`, and `wrap` print the updated file to stdout unless `--write` is set. With `--write`, they atomically replace the file and print `OK`.

`tree`, `replace`, `insert`, and `fmt` accept `--diff` to show a line-based diff on stderr. `tree` accepts `--depth N` for sub-form navigation:

```bash
lisp-sitter tree src/foo.el --depth 2
# → defun:my-func@12:1
# →   let:bindings@15:3
# →   if:condition@18:3
```

File arguments accept glob patterns and directories for batch operations:
```bash
lisp-sitter check "src/**/*.el"
lisp-sitter fmt lib/
lisp-sitter remove "*.lisp" dead-func --write
```

Exit code `0` on success, `1` on error.

## Agent workflow

For `.el`, `.lisp`, `.cl`, `.scm`, `.ss`, `.sld` files, prefer **lisp-sitter** over line-based edits:

1. `lisp-sitter tree PATH` — see what forms exist
2. `lisp-sitter get PATH SYMBOL` — read the full form text (optional)
3. `lisp-sitter replace PATH SYMBOL` or `substitute` — pass **complete** form text
4. `lisp-sitter check PATH` — validate after refactors

For structural restructuring: `wrap`, `extract`, `move`, and `rename`.

When writing new forms, use `complete` to fix unbalanced parens, then `fmt` to re-indent.

Example rule for `CLAUDE.md` or Cursor rules:

```markdown
For Lisp files (.el .lisp .cl .scm .ss .sld), use lisp-sitter for edits:
tree → get → replace/substitute (complete forms only) → check.
Use complete for paren balancing, fmt for indentation, rename for renaming.
Do not use line-based search/replace on structural Lisp files.
```

## MCP server

Expose the same structural tools to Cursor, Claude Code, or any MCP client:

| Tool | Description |
|------|-------------|
| `check_structural_file` | Validate a whole file (with `semantic: true` for deep checks) |
| `check_structural_node` | Validate one top-level form |
| `structural_tree` | Outline of top-level forms (`depth` for sub-forms) |
| `structural_bounds` | Byte range `START:END` for a symbol |
| `structural_get` | Full text of a named form |
| `structural_context` | Complete structural context: tree + bounds + full text |
| `structural_replace` | Replace a form (validates and saves) |
| `structural_insert` | Insert after `__start__`, `__end__`, or a symbol |
| `structural_complete` | Append missing `)` to an unbalanced form |
| `structural_format` | Re-indent a file (depth-based) |
| `structural_eval` | Run dialect-specific validation (byte-compile, sbcl, guile…) |
| `structural_remove` | Remove a form (with `keep_calls` option) |
| `structural_move` | Move a form after an anchor |
| `structural_substitute` | Replace a sub-expression inside a form |
| `structural_extract` | Extract a sub-expression into a new function |
| `structural_rename` | Rename a form and its call sites |
| `structural_wrap` | Wrap a form's body in a construct |

All tools accept `write: true` to save in place. `structural_replace`, `structural_insert`, and `structural_format` accept `diff: true` to show a line-based diff before applying.

Install into Cursor (default) or Claude:

```bash
make install
lisp-sitter mcp install              # ~/.cursor/mcp.json
lisp-sitter mcp install --claude     # ~/.claude/settings.json
lisp-sitter mcp install --cursor --claude
```

Manual config entry:

```json
{
  "mcpServers": {
    "lisp-sitter": {
      "command": "/path/to/lisp-sitter",
      "args": ["mcp", "serve"]
    }
  }
}
```

Run standalone: `lisp-sitter mcp serve` (stdio transport).

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

### Tests

```bash
cargo test                          # unit tests (169+)
cargo test -p lisp-sitter           # main crate only
```

For coverage reports (optional — not required for development):

```bash
cargo install cargo-llvm-cov
cargo llvm-cov --workspace --ignore-filename-regex "tests/explore" --open
```

External Lisp interpreters (emacs, sbcl, guile) are not required — the `eval` module uses a mockable `Runner` trait. No external test framework is needed for basic testing.

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
