# Usability review of the language (gate G1)

Method: one author wrote the fifty programs of `examples/corpus/` and the standard
library from the specification alone, then listed every point where the first attempt
failed. Each entry says what happened, whether the language or the tooling changed, and
what is left. A second reviewer (Martin) has not done this pass yet; it needs a person
who did not write the specification.

## Found while writing the corpus and the library

| # | First attempt | Message or result | Decision |
|---|---|---|---|
| 1 | `task loop every 10 ms` | `NE0003` expected a name, found keyword `loop` | Kept. The message names the keyword. A helpful hint (use backticks or another name) is a candidate for `NE0003`. |
| 2 | `Ok((raw / 10) as u8)` | `NE0204` lossy conversion, with the fixes `.wrap()`, `.saturate()`, `.try_into()` | Kept. The message is clear; the example was changed to keep the wider type. |
| 3 | `a |> f() == 30` | `NE0003` expected end of line, found `==` | Kept. The pipeline has the lowest precedence (spec 04 §4.6); bind it with `let` first. A hint could say so. |
| 4 | `if i as u8 < self.count:` | `NE0003` expected `>`, found `:` | Open. `u8 <` is read as the start of generic arguments. Write `(i as u8) < x`. The parser could look further ahead before choosing. |
| 5 | `use std.test.asserts` | `NE0003` expected a name, found keyword `test` | Fixed. Path segments after the first may now be reserved words (spec 04 §4.7). |
| 6 | `drive(start, true)` for a PLC input | the test faulted with `ContractViolation` | Open. For a PLC input the name has to be quoted: `drive("start", true)`. The error should say so. |
| 7 | `for i in 0..16: if i < self.count` | `NE0208` mismatch between `index` and `u8` | Kept. The loop variable has type `index`; convert explicitly. |
| 8 | a field named `set` in a `class` | not tried, because `set` is a keyword in `plc` | Avoided with `set_input`. |

## What worked at the first attempt

The three timers, the counters, the latches, the PID with anti-windup and the filters
type-checked and passed their own tests on the first run, and agree with the native
blocks of the interpreter on random inputs. Units caught no mistakes in the corpus
because the programs were written carefully; the dimension tests in the analyzer cover
the failure cases.

## Still to do before the freeze is final

- A reviewer who has not read the specification writes five programs and reports where
  they were stuck (human action, tracked in `docs/human-actions.md`).
- Items 4 and 6 above need a decision: parser change or clearer message.
