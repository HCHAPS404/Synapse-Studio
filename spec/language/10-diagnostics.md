# 10 · Diagnostics

Every diagnostic has: a code, a severity, the exact source span, a one-line message
in plain English (Spanish translation available), an explanation, and when possible
a suggested fix that tools can apply. `neuron explain NE0201` prints the long form.

```
error[NE0201]: cannot add a length to a duration
  --> motion.neuron:14:19
   |
14 |   let t = travel + 250 ms
   |           ------   ^^^^^^ duration (time)
   |           |
   |           length (mm)
   = help: did you mean `travel / speed + 250 ms`?
```

`NE` codes are errors, `NL` codes are lints (warnings, configurable).

## 10.1 Families

| Range | Area |
|---|---|
| NE00xx | lexical and syntax |
| NE01xx | names, modules, patterns |
| NE02xx | types, units, numeric |
| NE03xx | effects, resources, ownership, recursion |
| NE04xx | time, deadlines, loops, scheduling |
| NE05xx | dialects |
| NE06xx | hardware, targets, memory |
| NE07xx | cells, links, clock domains |
| NE08xx | safety and electrical |
| NE09xx | verification |

## 10.2 Initial catalogue

| Code | Message |
|---|---|
| NE0001 | tab character used for indentation |
| NE0002 | dedent does not match any enclosing indentation level |
| NE0003 | unexpected token |
| NE0004 | unterminated string or block comment |
| NE0005 | expected an indented block after `:` |
| NE0006 | invalid numeric literal (for example a number glued to its unit: `5ms`) |
| NE0007 | invalid escape sequence |
| NE0008 | invalid character or byte literal (non-ASCII or empty) |
| NE0009 | unexpected indentation at top level |
| NE0010 | unknown dialect in module header |
| NE0011 | unclosed bracket |
| NE0012 | contract (`requires`, `ensures`, `invariant`) outside the start of a function body |
| NE0013 | content after the `cell` block in a cell file |
| NE0014 | a number and its unit are not separated by exactly one space |
| NE0015 | non-ASCII character in a name |
| NE0016 | invalid `{...}` interpolation in a string |
| NE0101 | unknown name |
| NE0102 | name already defined in this scope |
| NE0103 | non-exhaustive `match` |
| NE0104 | circular import |
| NE0105 | private item used outside its module |
| NE0106 | import breaks the standard-library layering |
| NE0107 | `neuron.lock` is stale or its content hash does not match |
| NE0201 | incompatible dimensions |
| NE0202 | unit expected / unit not allowed here |
| NE0203 | invalid operation on an affine unit |
| NE0204 | lossy `as` conversion; use the suggested method |
| NE0205 | literal out of range for its type |
| NE0206 | unknown unit |
| NE0207 | arithmetic overflow proven at compile time |
| NE0208 | type mismatch |
| NE0301 | activity accesses a resource it declared it does not use |
| NE0302 | recursion not allowed in this dialect |
| NE0303 | `mut` argument aliases another argument or a global |
| NE0304 | `var` shared across cores; use a channel or signal |
| NE0305 | reference escapes its call |
| NE0401 | period is not a multiple of the base tick |
| NE0402 | task can miss its deadline |
| NE0403 | loop without a static bound in a real-time dialect |
| NE0404 | blocking operation in a task or interrupt |
| NE0405 | hard deadline requested on a soft real-time target |
| NE0501 | construct not allowed in this dialect |
| NE0502 | combinational loop in `hdl` |
| NE0601 | hardware resource bound twice |
| NE0602 | register field access violates its access rights |
| NE0603 | pin does not support this function |
| NE0604 | memory budget exceeded |
| NE0605 | requested clock frequency not achievable |
| NE0606 | derived rate error above tolerance |
| NE0607 | unknown target, chip or board |
| NE0701 | link latency budget cannot be met |
| NE0702 | clock domain crossing without synchronizer |
| NE0703 | tag mapped to incompatible types or units on two devices |
| NE0801 | incompatible voltage levels without isolation or level shifting |
| NE0802 | output without a safe state |
| NE0803 | `on fault` drives an output |
| NE0804 | robot target outside workspace or joint limits |
| NE0805 | mutex zone entered without lock |
| NE0901 | verification property violated (counterexample attached) |
| NE0902 | state unreachable |
| NE0903 | deadlock: state or step without outgoing transition |

## 10.3 Initial lints

| Code | Message |
|---|---|
| NL0001 | naming style |
| NL0201 | unchecked float division in device code |
| NL0202 | default numeric type used on a target without FPU |
| NL0203 | software floating point cost |
| NL0301 | bare number carries a physical quantity in a public signature |
| NL0402 | WCET analysis unavailable for this target |
| NL0501 | large value copy |
| NL0601 | interrupt nesting not supported; priorities flattened |
