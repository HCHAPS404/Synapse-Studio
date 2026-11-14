# Design: Neuron language (edition 2026)

Status: DRAFT v0.1 (Task 5). Implements `requirements.md` R1 to R13.

## 1. Lexical structure

- Source is UTF-8. Line comments `#`, doc comments `##` (attached to the next item),
  block comments `#[ ... ]#` (nestable).
- Indentation-sensitive: the lexer emits `INDENT`/`DEDENT` tokens. Tabs are
  rejected (`NE0001`); indentation unit is 2 spaces (formatter enforced).
- Identifiers: `[A-Za-z_][A-Za-z0-9_]*`. Types and states use `UpperCamel`,
  everything else `snake_case` (lint, not error).
- Numeric literals: `42`, `0x2A`, `0b1010`, `1_000`, `3.14`, `1e-3`. Optional unit
  suffix separated by a space: `5 ms`, `24 V`, `0.5 mm/s`, `60 degC`, `50 %`.
- Strings `"..."` with escapes; raw strings `r"..."`.
- Operators: `+ - * / % ** == != < <= > >= and or not & | ^ ~ << >> |> -> => .. ..= @ ? :`.

### Reserved keywords (edition 2026)
`module cell use pub const let var signal type struct enum class interface impl fn
task every on when for while loop in break continue return if elif else match
fsm state sequence step until run timeout safety fault device link via within zone
mutex between requires ensures invariant deadline asm extern test assert true false
none self Self as where`

## 2. Grammar overview (EBNF, abbreviated)

```
file        = (module_decl | cell_decl) item* ;
module_decl = "module" path NEWLINE ;
cell_decl   = "cell" IDENT NEWLINE INDENT cell_item* DEDENT ;
item        = use | const | let | signal | type_decl | fn | task | fsm
            | sequence | safety | test | asm_block | extern_block ;
use         = "use" path ( "." "{" IDENT ("," IDENT)* "}" )? NEWLINE ;
const       = "const" IDENT (":" type)? "=" expr NEWLINE ;
signal      = "signal" IDENT ":" type ("=" expr)? NEWLINE ;
fn          = "pub"? "fn" IDENT generics? "(" params? ")" ("->" type)? contracts? ":" block ;
task        = "task" IDENT trigger ("(" task_attr ("," task_attr)* ")")? ":" block ;
trigger     = "every" expr | "on" expr ;
task_attr   = "prio" INT | "deadline" expr | "reads" names | "writes" names ;
fsm         = "fsm" IDENT ":" INDENT state+ DEDENT ;
state       = "state" IDENT ":" block ;            # transitions: `-> Next when cond`
sequence    = "sequence" IDENT ("on" IDENT)? ":" INDENT step+ DEDENT ;
step        = "step" IDENT ":" stmt_list ;
safety      = "safety" ":" INDENT when_stmt+ DEDENT ;
when_stmt   = "when" expr ("for" expr)? ":" block ;
cell_item   = device | link | zone | mutex | sequence | safety | place ;
device      = "device" IDENT ":" path ("@" expr)? NEWLINE ;
link        = "link" expr "->" expr "via" expr ("every" expr)? ("within" expr)? NEWLINE ;
expr        = pipeline ;
pipeline    = range ( "|>" call_like )* ;
```

The full grammar lives in `spec/grammar/neuron.ebnf` once frozen.

## 3. Type system

- Nominal `struct`/`enum`/`class`; structural tuples.
- Units are a dimension vector over SI base units (s, m, kg, A, K, mol, cd) plus
  angle and ratio pseudo-dimensions; each quantity type is `Q<number_type, dim, scale>`.
  `degC` is an affine unit: only differences (`delta degC`) add freely.
- Numeric promotion is explicit except literal widening. Saturating operators
  `+|`, `-|`; wrapping operators `+%`, `-%`. Default overflow behavior is a compile
  error when provable, otherwise a trap that enters the safe state (configurable).
- References: `&x` shared, `&mut x` exclusive. Borrow regions are lexical and
  checked per function (simplified model, no lifetime annotations in edition 2026).
- `Option<T>` and `Result<T, E>` with `?` propagation.

## 4. Concurrency and time

- Each task gets a static priority. Every shared resource gets a ceiling equal to the
  highest priority of the tasks that declare it. Access is via `lock r: ...` blocks;
  the scheduler raises the running priority to the ceiling (SRP), which guarantees
  freedom from deadlock and from data races on single-core targets. Multi-core
  targets use per-core partitions plus message passing.
- Logical execution time (LET): inputs sampled at period start, outputs published at
  period end. This matches the cell-level Standard Time Model (spec `time-model`).

## 5. Profiles

| Profile | Allowed | Disallowed |
|---|---|---|
| `bare_metal_mcu` | everything static, `asm`, registers, interrupts | recursion, heap, `f64` without FPU (soft-float warning) |
| `plc` | scan-cycle tasks, IEC types, FB classes, sequences | `asm`, recursion, dynamic arrays |
| `hdl` | synthesizable subset: `bit<N>`, registers, `every clock`, combinational fns | loops without static bound, floats, `run` |
| `robot` | motion statements, points, frames, sequences | `asm`, interrupts |
| `host` | everything including `pool` heap and OS I/O | none |

## 6. Diagnostics

`NE0xxx` lexical/syntax, `NE01xx` names, `NE02xx` types/units, `NE03xx` effects and
resources, `NE04xx` timing, `NE05xx` profiles, `NE06xx` hardware/manifests,
`NE07xx` cell/links, `NE08xx` safety, `NE09xx` verification.

## 7. Examples

See `examples/thermostat.neuron` and `examples/bottling_line.neuron`.

## 8. Open questions (to resolve before freezing)

1. Keep `#` for comments or use `--`? (`#` chosen for familiarity; `@` reserved for placement.)
2. Should `var` (mutable) vs `let` (immutable) be kept, or `let mut`? Draft: `let` / `var`.
3. Units syntax for compound units: `mm/s` vs `mm per s`. Draft: `mm/s`.
4. Indentation vs braces: indentation chosen (R12); braces never accepted.
