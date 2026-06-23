# lisp-sitter usage guide

## Installation

```bash
make install              # build + install to ~/.cargo/bin
cargo install --path crates/lisp-sitter  # same without make
```

## Shell completions

```bash
eval "$(lisp-sitter completions bash)"   # bash
eval "$(lisp-sitter completions zsh)"    # zsh
lisp-sitter completions fish | source    # fish
```

## Quick reference

### Navigation

```text
tree PATH [--depth N]       # list top-level forms (or sub-forms with --depth)
bounds PATH SYMBOL          # byte range of a form
get PATH SYMBOL             # print form text
callers PATH SYMBOL         # find function call sites
```

### Editing

```text
replace PATH SYMBOL --body FORM  # replace a form
insert PATH AFTER --node FORM    # insert after a symbol, __start__, or __end__
substitute PATH SYMBOL --pattern P --replacement R  # replace sub-expression
remove PATH SYMBOL               # remove a form
move PATH SYMBOL --after ANCHOR  # reorder forms
wrap PATH SYMBOL --in TYPE       # wrap body in progn/let/if
extract PATH SYMBOL --pattern P --name N  # extract sub-expression to new function
rename PATH OLD NEW              # rename form + call sites
flatten PATH SYMBOL              # inline single-use function
convert-let PATH SYMBOL --to TYPE  # let <-> let*
instrument PATH SYMBOL --with FORM  # add tracing to body
```

### Validation & formatting

```text
check PATH [--semantic]      # structural + semantic validation
check-node --lang L --body F  # validate one form
eval PATH                    # run native compiler/checker
fmt PATH [--write]           # re-indent file
complete --body FORM         # close missing parens
```

### Config file

`~/.lisp-sitter.json` (or `~/.config/lisp-sitter/config.json`):

```json
{
  "extensions": {
    ".clef": "commonlisp",
    ".wl": "elisp"
  }
}
```

Also respects `LISP_SITTER_LANG` env var and `--lang` global flag.

### Safety features

- `--diff` : show line-based diff on stderr before changes
- `--confirm` : show diff + prompt "Apply? [y/N]" before writing
- `--json` : machine-readable JSON output for tree, callers
- Auto-backups : previous file saved to `$TMPDIR/lisp-sitter-backups/` before every write
- Atomic writes : temp file + rename, never corrupts on crash
- Glob & batch : `lisp-sitter check "**/*.el"` or `lisp-sitter check src/`

### Batch operations

All path arguments accept globs and directories:

```bash
lisp-sitter tree "src/**/*.lisp"
lisp-sitter fmt lib/
lisp-sitter check "*.scm" --semantic
```

### Git hook

```bash
lisp-sitter init-git-hook
```

Installs a pre-commit hook that runs `lisp-sitter check` on all staged Lisp files.

### MCP server

For Cursor, Claude Code, or any MCP client:

```bash
lisp-sitter mcp install              # ~/.cursor/mcp.json
lisp-sitter mcp install --claude     # ~/.claude/settings.json
lisp-sitter mcp serve                # run standalone (stdio)
```

See the README for the full list of 22 MCP tools.

---

## Architecture

```
lisp-sitter/
  crates/
    lisp-sitter-core/    # Plugin trait, edit engine, sexp fallback scanner
    lisp-sitter-elisp/   # Emacs Lisp plugin (tree-sitter grammar)
    lisp-sitter-cl/      # Common Lisp plugin
    lisp-sitter-scheme/  # Scheme plugin
    lisp-sitter/         # CLI binary + MCP server
```

The edit flow for every operation:

1. Parse with tree-sitter
2. Navigate to the target form (by name or pattern)
3. Splice replacement text at byte positions
4. Re-parse to validate structural correctness
5. If valid, write atomically (temp file + rename)

If tree-sitter fails (incomplete code), a hand-written sexp scanner provides fallback bounds and validation.
