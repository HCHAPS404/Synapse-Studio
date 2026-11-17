# 13 · Canonical formatting

`neuron fmt` rewrites source into one canonical layout. There are no options: two
developers formatting the same program get the same bytes. This matters more in
Neuron than in most languages because the same source is edited as text, as blocks
and as diagrams, and every editor must produce a clean diff.

## 13.1 Rules

| Area | Rule |
|---|---|
| Encoding | UTF-8, LF line endings, one final newline, no BOM |
| Indentation | 2 spaces per level, never tabs (a tab is `NE0001`) |
| Line width | 88 columns target. The formatter breaks lines that exceed it and never joins lines that fit |
| Trailing whitespace | removed |
| Blank lines | one between items, none after a block header, at most one inside a block, none at the start or end of a file |
| Imports | one `use` per module path, sorted alphabetically, `std` first, then packages, then `self` |
| Operators | one space around binary operators, none after unary operators |
| Units | exactly one space between a number and its unit (`5 ms`, `NE0014` otherwise). `%` follows the same rule (`35 %`) |
| Commas | one space after, none before. A trailing comma is added when a list is broken over several lines |
| Brackets | no space inside `()`, `[]`, `<>` |
| Alignment | the formatter aligns the `@` column of consecutive `io` bindings and the `=` column of consecutive `let` bindings only inside `inputs:`, `outputs:` and `io` groups |
| Comments | `#` starts a comment followed by one space. A comment keeps its attachment (its own line stays on its own line, a trailing comment stays on its line). `##` doc comments stay directly above their item |
| Literals | numbers keep their written base and digit grouping. Hex digits are lowercase, the prefix is lowercase (`0xff`) |
| Strings | no change inside strings |

## 13.2 Line breaking

When a construct does not fit in 88 columns the formatter breaks it as follows.

**Calls and constructors.** One argument per line, indented one level, with a
trailing comma:

```text
let heater = pwm(
  freq: 1 kHz,
  polarity: high,
  dead_time: 200 ns,
)
```

**Pipelines.** One stage per line, each starting with `|>`:

```text
let temp = adc.read()
  |> scale(0..4095 -> 0..3.3 V)
  |> lowpass(cutoff: 2 Hz)
```

**Long conditions.** Break before `and` and `or`:

```text
if level > 90 %
    and not manual
    and pump.running:
  pump.off()
```

**Signatures.** Parameters one per line when the signature exceeds the width, with the
return type on the closing line.

## 13.3 Properties

| Id | Property |
|---|---|
| P-FMT-1 | **Idempotence.** `fmt(fmt(x)) == fmt(x)` for every source `x` that parses |
| P-FMT-2 | **Meaning preservation.** The token stream of `fmt(x)`, with layout tokens removed, equals that of `x`, except for added or removed trailing commas |
| P-FMT-3 | **AST preservation.** `outline(parse(fmt(x))) == outline(parse(x))` |
| P-FMT-4 | **Comment preservation.** Every comment of `x` appears exactly once in `fmt(x)`, attached to the same item |
| P-FMT-5 | **Totality.** The formatter never fails on a source that parses, and never changes a source with syntax errors |
| P-FMT-6 | **Stability under editing.** Changing one statement changes only the lines of that statement and, when it is reflowed, the lines of its enclosing expression |

P-FMT-1 to P-FMT-3 are tested with property-based tests over generated programs
(a program generator driven by the grammar) and over the whole example corpus.
P-FMT-4 is tested with a generator that inserts comments at every legal position.

## 13.4 Formatting from other representations

The same layout engine is used when Neuron source is *produced* by a tool, so
generated code is already canonical:

- importers emit through the formatter;
- the block and diagram editors serialize through it;
- `neuron explain --fix` and the AI assistant propose edits as diffs against the
  canonical form.

A generated file is never accepted without being formatted.

## 13.5 Lint levels versus format

Formatting never reports a problem; it only rewrites. Style that cannot be
fixed by rewriting (names, unused items, missing contracts) belongs to the linter
(`NLxxxx` codes, chapter 10). The linter does not check anything the formatter
already guarantees.

## 13.6 Command line

| Command | Behavior |
|---|---|
| `neuron fmt <paths>` | rewrite files in place |
| `neuron fmt --check` | exit non-zero and print a diff when a file would change; used in CI |
| `neuron fmt --stdin` | read from standard input, write to standard output; used by editors |
| `neuron fmt --range L1:L2` | format only the lines in a range; used for format-on-type |

Files that contain syntax errors are left untouched and the parser diagnostics are
printed.

## 13.x Status of the reference formatter (v0.5, A11)

The reference formatter (
euron fmt, crates/neuron-syntax/src/format.rs) is
token based. It changes only spacing, indentation and blank lines, so comments are
never lost. Before returning it re-parses its own output and compares the token
signature with the input. If they differ, it returns the input unchanged and reports
that it fell back.

Not implemented yet: line breaking at 88 columns and 
euron fmt --range. Both are
scheduled after the freeze and do not change the rules above.
