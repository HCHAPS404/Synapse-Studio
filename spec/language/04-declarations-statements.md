# 04 · Declarations and statements

## 4.1 Module-level items

| Item | Form | Meaning |
|---|---|---|
| Import | `use std.control.{pid, lowpass}` / `use mylib.motors as m` | bring names into scope |
| Constant | `const target = 60 degC` | compile-time value, evaluated by the compiler |
| Static state | `var count: u32 = 0` | one instance for the program lifetime |
| Signal | `pub signal temp: f32 degC` | time-consistent value with LET semantics (06), exported to the tag dictionary when `pub` |
| Type | `struct` / `enum` / `class` / `interface` / `type Alias = ...` | (03) |
| Function | `fn name(...) -> T:` | (4.2) |
| Task / interrupt / rule | `task`, `on interrupt`, `when` | (06) |
| State machine / sequence | `fsm`, `sequence` | (06) |
| Hardware | `io`, `clocks`, `uses`, `extern` | (07) |
| Safety | `safety:` | (08) |
| Test | `test "name":` | compiled only by `neuron test` |

Items are private to their module unless marked `pub`. Attributes precede an item on
their own line: `@inline`, `@overflow(saturate)`, `@when(target.fpu != none)`,
`@deprecated("use x")`.

## 4.2 Functions

```neuron
fn scale(raw: u16, from: range<u16>, to: range<f32 degC>) -> f32 degC:
  let ratio = (raw - from.start) as f32 / (from.end - from.start) as f32
  return to.start + ratio * (to.end - to.start)

fn square(x: f32) -> f32 = x * x          # expression body
```

- Parameters are **read-only** by default. `mut p: T` passes `p` exclusively and
  allows modification; the call site repeats the marker: `fill(mut buffer)`.
- Default values and named arguments: `pwm.ch(1, freq: 1 kHz)`. Positional
  arguments come before named ones.
- Methods take `self` (read-only) or `mut self` as first parameter.
- Functions SHALL NOT be recursive in `mcu`, `plc`, `hdl` and `robot` (`NE0302`).
- A function used in a task with a deadline gets a WCET estimate (06).

### Contracts

Contracts are the first lines of a function body (anywhere else is `NE0012`;
`invariant` belongs in class bodies):

```neuron
fn set_speed(mut m: Motor, v: f32 mm/s):
  requires 0 mm/s <= v <= m.max_speed
  ensures m.speed == v
  m.speed = v
```

`requires`, `ensures` (with `result` and `old(expr)`) and `invariant` (in classes)
are checked statically when possible and otherwise compiled as runtime checks whose
failure raises `fault ContractViolation`. `@contracts(off)` removes runtime checks in
release builds of `host` only; device dialects always keep them.

## 4.3 Statements

```neuron
let x = 3                  # immutable binding
var y: u8 = 0              # mutable binding
y += 1
led.write(true)            # expression statement
return value
break / continue
assert speed <= limit, "speed limit"
fault Overheat             # enter safe state with a named fault (08)
emit start_requested       # raise an event (06)
```

### Conditionals

```neuron
if level > 90 %:
  pump.off()
elif level < 10 %:
  pump.on()
else:
  pass_through()

let mode = if manual then Mode.Manual else Mode.Auto     # expression form
```

### Match

```neuron
match cmd:
  Command.Stop => motor.stop()
  Command.Move(p, v) if v > 0 mm/s =>
    planner.go(p, v)
  Command.Home => home()
```

Matches over enums and bounded integers SHALL be exhaustive (`NE0103`); `_` is the
wildcard.

### Loops

```neuron
for i in 0..8:            # static bound
  buf[i] = 0

for sample in samples:     # over an array or slice
  sum += sample

while not uart.ready() max 1000:    # bounded: at most 1000 iterations
  pass

loop max 3:               # retry up to 3 times
  if try_connect(): break
```

In `mcu`, `plc`, `hdl` and `robot`, every loop SHALL have a static bound: a range
with constant limits, a fixed-size collection, or a `max N` clause (`NE0403`).
When the bound is reached without `break`, the loop ends normally; the compiler
reports it in the WCET analysis. Unbounded `loop:` and `while` are legal only in
`host`.

### Atomic blocks

```neuron
atomic:
  snapshot = shared_counter
  shared_counter = 0
```

`atomic` executes the block without preemption by any task or interrupt that shares
the accessed resources (it raises the priority to the resource ceiling). The compiler
inserts the equivalent automatically where needed (06); writing it is optional
documentation.

## 4.4 Patterns

Patterns appear in `let`, `match` and `for`:
literals, ranges (`0..=9`), identifiers, `_`, tuples `(a, b)`, enum variants
`Command.Move(p, v)`, struct patterns `Point(x: px, y: _)`, and alternatives
`A | B`.

## 4.5 Errors and absence

- `T?` represents an optional value; `none` is its empty value.
- `Result<T, E>` represents success or an error; `E` is usually an `enum`.
- Postfix `?` propagates: inside a function returning `Result<_, E>` (or `_?`),
  `x?` returns early on error (or `none`).
- `x ?? default` returns the contained value or `default`.
- There are no exceptions. Unrecoverable conditions in device code use `fault`.

```neuron
fn read_temp(mut s: Sensor) -> Result<f32 degC, IoError>:
  let raw = s.read()?
  return Ok(raw as f32 degC)
```

## 4.6 Expressions and precedence

From lowest to highest:

| Level | Operators | Associativity |
|---|---|---|
| 1 | `\|>` pipeline | left |
| 2 | `if … then … else …` | right |
| 3 | `??` | right |
| 4 | `or` | left |
| 5 | `and` | left |
| 6 | `not` | prefix |
| 7 | `== != < <= > >= in` | chainable |
| 8 | `.. ..=` | none |
| 9 | `\|` | left |
| 10 | `^` | left |
| 11 | `&` | left |
| 12 | `<< >>` | left |
| 13 | `+ - +% -% +\| -\| ++` | left |
| 14 | `* / % *% *\|` | left |
| 15 | unary `- ~` | prefix |
| 16 | `**` | right |
| 17 | `as` | left |
| 18 | call `()`, index `[]`, field `.`, `?` | postfix |

**Mapping** `a -> b` appears only as an argument and builds a linear map between
two ranges: `scale(0..4095 -> 0..120 degC)`.

**Lambdas** for higher-order library functions: `fn(x) => x * 2`. They capture
read-only and are inlined; they cannot escape their call.

**Construction** of structs and classes uses call syntax with named arguments:
`Point(x: 10 mm, y: 5 mm)`, `ton(preset: 3 s)`.

## 4.7 Modules and packages

- `use a.b.c` imports module `c`; `use a.b.{c, d}` imports names; `as` renames. A
  segment after the first MAY be a reserved word, so that `use std.test.asserts` is legal.
- `std` is the standard library; `self.` refers to the current package root.
- Circular imports are an error (`NE0104`).
- `neuron.toml`:

```toml
[package]
name = "bottling"
version = "0.1.0"
edition = "2026"

[dependencies]
displays = { git = "https://example.org/neuron-displays", tag = "v0.3.0" }

[targets.ctl]
board = "nucleo_f446re"
```

Conditional compilation: `@when(target.arch == "avr")`, `@when(target.has(fpu))`,
`@when(target.periph(uart) >= 2)`.
