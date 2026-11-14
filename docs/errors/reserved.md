# Reserved diagnostics

These codes are part of the catalogue (`spec/language/10-diagnostics.md`) but their
analyses belong to later tasks. They are documented now so that the numbers are stable.

| Code | Meaning | Implemented by |
|---|---|---|
| NE0106 | an import breaks the standard-library layering (spec 12 §12.1) | A12 |
| NE0107 | `neuron.lock` is stale or a content hash does not match | package manager |
| NE0601 | hardware resource bound twice | B1 target model |
| NE0602 | register field access violates its access rights | B1 |
| NE0603 | pin does not support the requested function | B1 |
| NE0605 | requested clock frequency is not achievable | B1 (clock solver) |
| NE0606 | derived rate error above the tolerance | B1 |
| NE0607 | unknown target, chip or board | B1 |
| NE0701 | link latency budget cannot be met | D3 |
| NE0702 | clock-domain crossing without a synchronizer | B7 |
| NE0703 | tag mapped to incompatible types or units on two devices | D3 |
| NE0801 | incompatible voltage levels without isolation or level shifting | D4 |
| NE0804 | robot target outside the workspace or the joint limits | C3, D4 |
| NE0805 | mutex zone entered without a lock | D4 |
| NL0001 | naming style | A11 (linter) |
| NL0201 | unchecked float division in device code | A11 |
| NL0202 | default numeric type used on a target without FPU | B1 |
| NL0203 | software floating-point cost | B1 |
| NL0301 | bare number carries a physical quantity in a public signature | A12 |
| NL0501 | large value copy | A11 |
| NL0601 | interrupt nesting not supported; priorities flattened | B1 |
