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

Read values from stdin with `--body-file -` or `--node-file -`:

```bash
echo '(defun foo (x)' | lisp-sitter complete --lang elisp --body-file -
# → (defun foo (x))
```

### Edit workflow

Get a form, pipe a replacement, then format:

```bash
echo '(defun greet (name) (message "hi" name))' > greet.el
echo '(defun greet (name)\n(message "hello, %s" name))' | \
  lisp-sitter replace greet.el greet --body-file - --write
lisp-sitter fmt greet.el --write
```

### Shell completions

Tab-complete subcommands in your terminal:

```bash
eval "$(lisp-sitter completions bash)"   # bash
eval "$(lisp-sitter completions zsh)"    # zsh
lisp-sitter completions fish | source    # fish
```

## Commands

| Command | Description |
|---------|-------------|
| `tree PATH` | Outline of top-level definitions, one per line (`defun:foo@12:1`). `--all` also lists non-definition forms (`require`, `provide`, `setq`, …) |
| `bounds PATH SYMBOL` | Byte positions `START:END` for a named form |
| `get PATH SYMBOL` | Print the full text of a named top-level form |
| `replace PATH SYMBOL` | Replace a form; requires `--body` or `--body-file` |
| `insert PATH AFTER` | Insert a form; requires `--node` or `--node-file` |
| `complete` | Append missing `)` to an unbalanced s-expression |
| `fmt PATH` | Re-indent a file (depth-based, 2-space indent). `--write` to save. `--align` for continuation-line alignment (arg-column instead of depth×2) |
| `eval PATH` | Run dialect-specific validation (byte-compile, sbcl, guile…) |
| `remove PATH SYMBOL` | Remove a form; optionally replace call sites with ignore |
| `move PATH SYMBOL --after ANCHOR` | Reorder a form after another symbol, `__start__`, or `__end__` |
| `substitute PATH SYMBOL` | Replace a sub-expression inside a form using `--pattern` / `--replacement` |
| `extract PATH SYMBOL` | Extract a sub-expression into a new function |
| `rename PATH OLD NEW` | Rename a form, its call sites, and `#'old`/`'old` references. `PATH` may be a file, directory, or glob for a **project-wide** rename (definition + every reference across all matching files). `--refs` also renames plain `'old`; `--no-refs` renames only head-position call sites |
| `wrap PATH SYMBOL` | Wrap body in `progn`, `let`, or `if` |
| `analyze PATH` | Project-wide semantic analysis over a directory or glob: unused definitions, unresolved calls, and arity mismatches. `--unused` / `--unresolved` / `--arity` to run a subset (default: all) |
| `callers PATH SYMBOL` | Callers of a symbol. Single file, or project-wide when `PATH` is a directory/glob |
| `callees PATH SYMBOL` | Symbols called directly from a definition's body across `PATH` |
| `explore PATH SYMBOL` | Source + definitions + callers + callees for one symbol |
| `impact PATH SYMBOL` | Transitive callers (blast radius). `--depth N` (default 5) |
| `diff REF PATH` | Git diff since `REF` → touched symbols in `PATH`. `--impact` adds blast radius per symbol |
| `check PATH` | Validate file → `OK` or syntax error on stderr |
| `check PATH --semantic` | Deep validation — docstrings, missing `provide`/`in-package`/library export warnings (elisp, commonlisp, scheme) |
| `check-node` | Validate one form; `--lang elisp\|commonlisp\|scheme` |
| `mcp serve` | Run MCP server on stdio |
| `mcp install` | Add server to `~/.cursor/mcp.json` (or `--claude-code`, `--claude-desktop`) |

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

### Safety

- `--diff` — show line-based diff on stderr before changes
- `--confirm` — show diff + prompt before writing
- Auto-backups — previous version saved to `$TMPDIR/lisp-sitter-backups/`
- Atomic writes — temp file + rename, never corrupts on crash

### Configuration

Language is inferred from file extension. Override with `LISP_SITTER_LANG=elisp|commonlisp|scheme` or the `--lang` global flag.

Custom extension mappings and project-specific definer macros can be set in
`~/.lisp-sitter.json` or `~/.config/lisp-sitter/config.json`:

```json
{
  "extensions": {
    ".foo": "elisp",
    ".bar": "scheme"
  },
  "extra_definers": {
    "elisp": ["define-widget", "transient-define-prefix"],
    "commonlisp": ["define-app-command"]
  }
}
```

`extra_definers` registers additional top-level definition forms per language
(`elisp`, `commonlisp`, `scheme`) so your own def-macros are listed by `tree` and
addressable by `bounds`/`get`/`replace`/`rename`. Each is treated like `defun`/
`define` — the name is the second element.

## Project-wide operations

`rename`, `analyze`, and the call-graph commands operate across a whole project when given a directory or glob.

```bash
# Rename a function and every call site across the project (preview, then apply)
lisp-sitter rename src/ old-name new-name
lisp-sitter rename src/ old-name new-name --write

# Call graph navigation (scan-on-demand, no index file)
lisp-sitter callers src/ my-func
lisp-sitter callees src/ my-func
lisp-sitter explore src/ my-func
lisp-sitter impact src/ my-func --depth 5

# Git-aware: symbols touched by changes since main (+ optional blast radius)
lisp-sitter diff main src/
lisp-sitter diff main src/ --impact

# Semantic analysis: unused definitions, unresolved calls, arity mismatches
lisp-sitter analyze src/
lisp-sitter analyze "src/**/*.el" --unused --arity
```

`analyze` reports three classes of issue:

- **unused** — a `defun`/`define`/macro with no references anywhere in the analyzed files
- **arity** — a call whose argument count does not match the definition's lambda list (`&optional`/`&rest`/variadic are understood)
- **unresolved** — a call to a name that is neither defined in the analyzed files nor a known builtin

Unresolved-symbol detection is heuristic: it relies on a curated (non-exhaustive)
builtin table per dialect, so dynamically-built calls or builtins outside the
table may be reported. Treat the output as warnings, not errors.

## Editor integration (Emacs)

[`editor/lisp-sitter.el`](editor/lisp-sitter.el) is a thin Emacs wrapper that shells
out to the CLI, so interactive edits get the same parse-and-validate guarantees as
the command line.

```elisp
(add-to-list 'load-path "/path/to/lisp-sitter/editor")
(require 'lisp-sitter)
(add-hook 'emacs-lisp-mode-hook #'lisp-sitter-mode)
(add-hook 'scheme-mode-hook #'lisp-sitter-mode)
(add-hook 'lisp-mode-hook #'lisp-sitter-mode)
;; optional: validate with lisp-sitter after every save
(setq lisp-sitter-check-on-save t)
```

| Key | Command | Action |
|-----|---------|--------|
| `C-c s t` | `lisp-sitter-tree` | Outline of top-level forms |
| `C-c s g` | `lisp-sitter-get` | Show the text of a form |
| `C-c s r` | `lisp-sitter-replace-defun` | Re-validate and rewrite the form at point |
| `C-c s R` | `lisp-sitter-rename` | Rename a symbol (`C-u` for project-wide) |
| `C-c s f` | `lisp-sitter-format-buffer` | Re-indent the file |
| `C-c s c` | `lisp-sitter-check` | Validate the file |
| `C-c s a` | `lisp-sitter-analyze` | Semantic analysis (`C-u` for project-wide) |

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
| `structural_rename` | Rename a form, call sites, and refs (`refs: true` for plain `'old`) |
| `structural_rename_project` | Rename a symbol across a directory or glob (definition + every reference); diff preview unless `write: true` |
| `structural_analyze` | Project-wide unused-definition, unresolved-call, and arity analysis over a directory or glob |
| `structural_callers` | Callers of a symbol (`path` = file, directory, or glob) |
| `structural_callees` | Direct callees from a symbol's definition(s) across the project |
| `structural_explore` | Source + callers + callees for one symbol |
| `structural_impact` | Transitive callers (blast radius); optional `depth` |
| `structural_diff` | Git diff since `ref` → touched symbols; `impact: true` for blast radius |
| `structural_wrap` | Wrap a form's body in a construct |

All tools accept `write: true` to save in place. `structural_replace`, `structural_insert`, and `structural_format` accept `diff: true` to show a line-based diff before applying.

Install into Cursor (default) or Claude:

```bash
make install
lisp-sitter mcp install                       # ~/.cursor/mcp.json
lisp-sitter mcp install --claude-code         # ~/.claude.json (Claude Code CLI)
lisp-sitter mcp install --claude-desktop      # ~/.claude/settings.json (Claude Desktop)
lisp-sitter mcp install --claude-code --claude-desktop  # both Claude targets
lisp-sitter mcp install --cursor --claude-code --claude-desktop  # all three
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
  lisp-sitter/         # CLI binary + MCP server + project analysis
editor/
  lisp-sitter.el       # thin Emacs wrapper around the CLI
```

## License

MIT — see [LICENSE](LICENSE).
