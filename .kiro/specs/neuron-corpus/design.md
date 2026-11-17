# Design: validation corpus and freeze (task A13)

## Layout

```text
examples/corpus/<dialect>/neuron.toml     one package per dialect
examples/corpus/<dialect>/<id>_<name>.neuron
```

Prefixes: `m` mcu, `p` plc, `c` core, `h` host, `d` hdl, `r` robot, `u` ui, `x` cell.

## Checks

- `tools/corpus-check.ps1 [-Fix]` runs `check`, `fmt --check` and `test` on each package
  and writes a short `out/corpus.log` (errors and results only).
- `crates/neuron-interp/tests/a13_corpus.rs` analyzes each file alone, runs it for two
  simulated seconds and runs its tests.
- CI runs the three commands for every folder.

## Rules for a corpus program

- Original, small (about 10 to 40 lines), one idea per program.
- Generic device profiles only; no vendor trademarks beyond descriptive board names.
- A `test` block where the behavior can be observed in the interpreter.
- No claim of certified safety.

## Freeze

The specification README states the version (v1.0-rc1), the decisions and the open
items of `docs/usability-review.md`. The tag `spec-v1.0-rc1` marks the commit.
