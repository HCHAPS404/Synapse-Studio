# Requirements: formatter, `explain` and language server (task A11)

Status: v1.0 (2026-11-16). Parent spec: `neuron-language`. Normative text:
`spec/language/13-formatting.md` and `docs/editor-support.md`.

## Context

The language can be checked (A4 to A8), exchanged (A9) and run (A10). A person still
needs three things to work with it every day: one canonical layout so that diffs stay
small whether the source is edited as text, as blocks or as diagrams; a way to read the
long explanation of any diagnostic; and an editor that shows the diagnostics while
typing.

## Requirements

| Id | Requirement | Acceptance |
|---|---|---|
| T1 | `neuron fmt` rewrites source into the canonical layout of chapter 13 | rules of 13.1 that do not need line breaking: indentation, spacing, commas, brackets, blank lines, comments, alignment of `@` |
| T2 | P-FMT-1: formatting is idempotent | on every example and every program of the specification |
| T3 | P-FMT-2 and P-FMT-3: the tokens and the structure do not change | same token list and same outline |
| T4 | P-FMT-4: every comment appears exactly once | comment count |
| T5 | P-FMT-5: total; a source with syntax errors is never changed | arbitrary text never panics; errors are reported and the file is left alone |
| T6 | Layout noise does not matter | extra spaces, trailing spaces and CRLF give the same output as the clean source |
| T7 | `neuron fmt --check` and `--stdin` | exit code 1 when a file would change; editors can pipe text through |
| T8 | `neuron explain NE####` | prints the section of `docs/errors/` for every documented code; unknown codes exit with 1 |
| T9 | A language server over standard input and output | diagnostics of the whole package on every change, hover (declaration, documentation and type), go to definition, document formatting |
| T10 | Editor support files | a VS Code / Kiro extension with a TextMate grammar and the language server client (installed by the user) |

## Out of scope

Breaking and joining lines at 88 columns (13.2), formatting a range (`--range`), the
linter (`NL` codes), rename and completion in the language server, a debug adapter.
