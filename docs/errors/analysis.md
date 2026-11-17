# Semantic errors (tasks A4 to A8)

Source of truth for the catalogue: `spec/language/10-diagnostics.md`. The rules are in
`spec/language/15-analysis.md`. `neuron explain NE0201` prints the matching section.

## Names and modules

### NE0101 unknown name
The name is not defined in any enclosing scope, in the module, in its imports or in the
prelude. The message suggests the closest known name. It also reports a missing module,
a missing item in a module, an unknown state in a transition and an unknown step in a
jump.

### NE0102 name already defined in this scope
Two definitions of one name in the same scope: items of a module, parameters, `let`
bindings of one block, states of a state machine, steps of a sequence, modules of a
package, or an import that clashes with a definition. Rename one of them.

### NE0103 non-exhaustive `match`
Some value has no arm. The message lists the missing enum variants or the first missing
integer value. Add the arms or a `_` arm. Arms with a guard (`if ...`) do not count.

### NE0104 circular import
Modules import each other. The message shows the cycle. Move the shared definitions to
a third module.

### NE0105 private item used outside its module
Mark the item `pub`, or use another item.

### NE0106 import breaks the standard-library layering
A module of the standard library (`std.<layer>...`) imports from a layer that comes
after its own in the table of spec 12.1 (core, units, time, logic, hal, control, ...).
Layers form a stack: move the shared code to an earlier layer.

### NE0108 invalid `neuron.toml`
The manifest needs `[package]` with `name` and `edition`.

## Types and units

### NE0201 incompatible dimensions
```text
let t = 5 mm + 2 s      # length plus time
if speed > 5 then ...   # a plain number is not a speed; write 5 mm/s (0 is allowed)
```
Convert or multiply so that both sides have the same dimension.

### NE0202 unit not allowed here
A unit follows a number, a tuple or a vector constructor (`xyz`, `xy`, `rpy`, `joints`).
For other calls write the unit on each argument.

### NE0203 invalid operation on an affine unit
`degC + degC` has no meaning. Add a temperature difference (`K`) or subtract two
temperatures to get one.

### NE0204 lossy conversion
`as` would lose information, or an integer unit conversion is not exact. Use `.round()`,
`.wrap()`, `.saturate()` or `.try_into()`, or a finer unit.

### NE0205 literal out of range
The literal does not fit the type (`let x: u8 = 256`).

### NE0206 unknown unit
The unit is not built in and no `unit` item defines it.

### NE0207 arithmetic overflow proven at compile time
A constant expression such as `200 + 100` does not fit its type. Use a wider type or
`+%` / `+|`.

### NE0208 type mismatch
The expected and the found types differ, a condition is not `bool`, a generic bound is
not met, or an operator is not defined for the operands.

### NE0209 wrong arguments in a call
Unknown, repeated, missing or surplus arguments. Positional arguments come before named
ones, and constructors take named arguments.

### NE0210 assignment to something that is not mutable
Declare the variable with `var`, the parameter with `mut`, the method with `mut self`, or
the field with `var`.

### NE0211 interface not implemented
A class that declares `is I` lacks a method of `I`, or a class that does not declare it is
used where `I` is expected.

### NE0212 no such field, method or variant
The message lists what exists.

### NE0214 control flow outside its construct
`break` and `continue` need a loop. `return value` needs a function.

### NE0215 `mut` marker mismatch
Write `f(mut x)` for a `mut` parameter and `f(x)` for a plain one.

### NE0216 constant index out of bounds
The index is outside the array or the bit vector.

### NE0217 missing return
Some path reaches the end of a function that returns a value.

## Dialects and effects

### NE0302 recursion not allowed in this dialect
The function can call itself, directly or through others. Use a loop with a bound.

### NE0303 `mut` argument aliases something
A place passed `mut` overlaps another argument or a global that the callee uses.

### NE0305 reference escapes its call
Slices are parameter views. They cannot be returned, stored in fields or variables, or
bound with `let`.

### NE0403 loop without a static bound
Use a range with constant limits, an array, or `max N` with a constant.

### NE0404 waiting in a task or interrupt
Tasks run to completion. Express waiting with a state machine, a sequence or an event.

### NE0405 hard deadline on a soft real-time target
`host` cannot guarantee deadlines. Place the logic on an `mcu` or `hdl` device.

### NE0501 construct not allowed in this dialect
The message names the dialects where it is legal.

### NE0502 combinational loop
Reserved for the `hdl` backend (task B7).

## Time, concurrency and memory

### NE0301 undeclared access
A task lists `reads` or `writes` and touches a resource outside them.

### NE0304 `var` shared across cores
Use a `channel` or a `signal`.

### NE0306 signal with several writers
Give each signal one writer activity.

### NE0401 period not a multiple of the base tick
Use a multiple of the tick, or a target with a finer timer.

### NE0402 deadline can be missed
The response time computed from the worst-case execution times exceeds the deadline.

### NE0406 deadline longer than the period
A release must finish before the next one.

### NE0407 not a constant duration
Periods, offsets, deadlines, `scan`, `watchdog` and `rate` are constants.

### NE0604 memory budget exceeded
The total of static data, stack and runtime exceeds the RAM, or a `stack` budget is
smaller than the computed need. The message lists the largest parts.

## Safety and verification

### NE0802 output without a safe state
Add it to `safe_state:` in a `safety:` block.

### NE0803 `on fault` drives an output
The safe state is already applied. `on fault` may only log or notify.

### NE0901 property violated
The message shows a shortest counterexample. The same trace is in the analysis report.

### NE0902 unreachable state
No path from the initial state leads to it.

### NE0903 state without a way out
Add a transition, or mark the state `final`. A state whose entry raises `fault` is
already terminal.

### NE0904 state machine without exactly one initial state
Mark one state `initial`.

## Warnings

| Code | Meaning |
|---|---|
| NL0402 | no timing model is configured, so deadlines are not analyzed |
| NL0403 | an interrupt without `@min_interval` is left out of the response-time analysis |
| NL0901 | a property could not be decided (inputs only, limits reached, or coarse time) |
