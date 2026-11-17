# Design: formatter, `explain` and language server (task A11)

Status: v1.0 (2026-11-16). Implements `requirements.md`.

## Components

| Where | Role |
|---|---|
| `crates/neuron-syntax/src/format.rs` | the formatter; works on the token stream |
| `crates/neuron-cli/src/explain.rs` | `explain`; the text of `docs/errors/` embedded at build time |
| `crates/neuron-lsp` | language server: `Server` (pure: message in, messages out) and the framing of messages |
| `crates/neuron-cli` | `neuron fmt`, `neuron explain`, `neuron lsp` |
| `editors/vscode` | extension: grammar and client of `neuron lsp` |

## Decisions

1. **Token-based layout, never tree-based.** The formatter changes only the white space
   between tokens, the indentation of lines and the blank lines. It cannot change the
   meaning of a program, and comments are never lost because they are tokens. A
   tree-based printer (like the NIR printer) would drop comments and spacing choices.
2. **Self-check.** After formatting, the result is lexed again and its token list is
   compared with the input; if they differ, or the result does not parse, the input is
   returned and `Formatted::fell_back` is set. The tests assert that it never happens on
   the corpus, so a bug cannot silently corrupt a file.
3. **Keep what is ambiguous.** The spacing of `+ - * / %`, `< >` and `..` is kept as
   written, because the same characters are operators, units, generics and ranges. Only
   operators that have one meaning (`=`, `==`, `->`, `|>`, `??` and the compound
   assignments) are forced to one space on each side. A space before `(` or `[` is kept,
   because it separates a call from the next expression.
4. **Indentation from the lexer's own layout tokens.** The depth of a line is the number
   of `Indent` minus `Dedent` tokens before it. Comment-only lines take the depth of the
   next code line, unless they were written at a still-open deeper level.
5. **No line breaking yet.** Chapter 13.2 describes how to break long lines. It needs the
   syntax tree to decide where, so it is left for later; the formatter never joins or
   breaks lines, which keeps it idempotent.
6. **The language server analyzes the whole workspace on each change.** Programs are
   small, the analysis takes milliseconds, and cross-file names (`use`) then always
   resolve. Hover and definition use the declaration (from the outline) and the type
   (from NIR when the package has no errors).
7. **The server core is a function from message to messages.** The transport (framing)
   is separate, so the protocol logic is tested without a process or a client.
8. **`explain` reads the same files as the documentation.** The test of the catalogue
   already guarantees that every code is documented; `explain` therefore works for every
   code.

## Tests

`crates/neuron-syntax/tests/format.rs`: P-FMT-1 to P-FMT-5 over the examples and the
specification, the rules of chapter 13, messy layouts (extra and trailing spaces, CRLF),
arbitrary text (proptest). `crates/neuron-lsp`: protocol tests of every request.
`crates/neuron-cli`: `explain` tests.
