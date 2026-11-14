# 15 · Semantic analysis

This chapter defines what the compiler checks after parsing, and in which order. It
covers tasks A4 to A8 of the roadmap and is implemented by the `neuron-sema` crate.
Every rule here has a diagnostic code (chapter 10) and a test.

## 15.1 Pipeline

```text
files ──parse──▶ modules, symbols, imports        (15.2)
      ──────────▶ name resolution in bodies        (15.2)
      ──────────▶ types, units, patterns           (15.3)
      ──────────▶ facts: bodies, accesses, calls   (15.4)
      ──────────▶ dialect rules and effects        (15.4)
      ──────────▶ time, concurrency, memory        (15.5)
      ──────────▶ safe state and verification      (15.6)
```

A file with syntax errors takes part in the package (its module name is known) but is
not analyzed further, so one syntax error never produces a cascade of semantic errors.
Diagnostics are sorted by file, position and code, so the output is deterministic.

### Gradual typing at the vocabulary boundary

The standard library (task A12) and the target descriptions (task B1) do not exist yet
when this chapter is first implemented. The checker is therefore **strict where the
program defines things** (its own types, functions, units and literals) and **gradual
where it uses external vocabulary** (function blocks such as `ton`, peripheral
operations, device manifests). A value of unknown type is compatible with everything and
produces no diagnostics. Tasks A12 and B1 replace the vocabulary with declarations and
tighten the check without changing any rule below.

## 15.2 Names and modules (task A4)

**Namespaces.** A module has one namespace for module-level names (functions, types,
constants, state, tasks, devices, ...). Local scopes nest inside it: parameters, `let`
and `var`, loop variables, pattern bindings, circuit members and state-machine variables.
Lookup order: local scopes (innermost first), module items, imports, prelude.

**Definitions** (`NE0102`). A name is defined once per scope. Shadowing in a nested
scope is allowed; redefinition in the same scope is not. States of one `fsm`, steps of
one `sequence` and module names of one package are also unique.

**Imports.**

| Form | Meaning |
|---|---|
| `use a.b.c` | the module `a.b.c`, or the item `c` of module `a.b`, bound to `c` (or the alias) |
| `use a.b.{c, d}` | the items `c` and `d` of module `a.b` |
| `use self.x.{y}` | `self` is the package root: module `<package>.x`, or `x` |
| `use std.<layer>...` | the standard library; the layer SHALL be one of chapter 12 §12.1 |
| `use <dep>...` | a dependency named in `[dependencies]` of `neuron.toml` |

An import of a missing module or item is `NE0101`. An item that is not `pub` cannot be
imported or reached as `module.item` from another module (`NE0105`). Importing a name
that the module also defines, or two different meanings for one name, is `NE0102`.
Names imported from `std` or from a dependency are *external vocabulary* (15.1).

**Cycles** (`NE0104`). The graph of imports between package modules SHALL be acyclic. The
diagnostic lists the cycle (`a -> b -> a`).

**Unknown names** (`NE0101`) are reported with the closest known name as a hint.
`NE0206` reports unknown units. Names that belong to the hardware or to a device
description (pins, peripheral instances, device addresses, `via` media, widget fonts and
colors) are vocabulary and are not resolved by this phase.

**Events** need not be declared: `emit E` and `on E` introduce `E`.

**Manifest** (`NE0108`). `neuron.toml` SHALL contain `[package]` with `name` and `edition`.

## 15.3 Types, units and patterns (task A5)

### Literals and defaults

An integer literal has the type its context requires and an integer range check
(`NE0205`). A constant expression that overflows its type is `NE0207`. Without context
the default is `i32` for integers and `f32` for reals. A real literal never becomes an
integer. The literal `0` is the only plain number that adopts any unit.

### Coercion

An expression of type `A` is accepted where `B` is expected when:

| Expected | Accepted |
|---|---|
| numeric type with unit `U` | numeric of the same dimension and kind, or a literal of any dimension-compatible unit; integers only when the conversion is exact (`NE0204`) |
| `T?` | `T`, `none`, `T?` |
| `[T; N]` | `[T; N]` (length checked); `[T]` accepts any array of `T` |
| `bit<1>` and `bool` | each other (comparison results in `hdl`) |
| `Interface` | a class that declares `is Interface` |
| struct, enum, class | the same definition with compatible type arguments |

Anything else is `NE0208`; a different dimension is `NE0201`.

### Arithmetic on quantities

| Operation | Result |
|---|---|
| `a + b`, `a - b`, comparisons | dimensions SHALL be equal (`NE0201`); the result has the unit of the left operand |
| `a * b`, `a / b` | units multiply or divide exactly |
| `degC - degC` | `K` (a difference) |
| `degC + K` | `degC` |
| `degC + degC` | `NE0203` |
| `%` | same dimension as the left operand |
| `x ** n` | `n` SHALL be a constant integer when `x` has a unit |
| `sin`, `cos`, `tan` | angle or plain number |
| `exp`, `log`, `asin`, `acos`, `atan` | plain number |
| `sqrt` | every exponent of the dimension SHALL be even |

> **Property UNIT-1** is tested at two levels: the unit algebra (`neuron_sema::units`)
> converts and combines quantities exactly, and the checker accepts `x + y` for two
> quantities exactly when their dimensions are equal.

### Conversions

`as` converts without losing information (`NE0204` otherwise): widening integers,
integers into floats that represent them exactly, `f32` into `f64`. It attaches a unit
to a plain number and converts between units of one dimension. It never removes a unit
(`NE0201`); divide by a unit first, for example `(t / 1 ms) as u32`.

### Calls and construction

Arguments are matched by position, then by name (`NE0209` for unknown, repeated, missing
or surplus arguments). Constructors of structs and classes take named arguments for every
field without a default. A `mut` parameter SHALL be passed as `mut x` and a plain
parameter without it (`NE0215`). A place passed `mut` or used as receiver of a `mut self`
method SHALL be mutable (`NE0210`).

### Generics

Generic functions and types are checked once, with their parameters opaque; the
arguments of a call bind the parameters. A bound is satisfied by the types that
implement it: `Ord` by integers, fixed point, bit vectors, durations, instants and
quantities (floats are only `PartialOrd`), and an interface by the classes that declare
it. Monomorphization happens when NIR is lowered (task A9).

### Classes and interfaces

A class that declares `is I` SHALL define every method of `I` with the same shape
(`NE0211`). Accessing a field or method that does not exist is `NE0212`.

### Statements

`break` and `continue` need a loop; `return` with a value needs a function (`NE0214`). A
function with a result SHALL return on every path (`NE0217`). A constant index outside
an array or bit vector is `NE0216`. Assigning to a `let`, to a read-only parameter, to a
`const` or to a plc input is `NE0210`.

### Match exhaustiveness (`NE0103`)

A `match` is exhaustive when an arm without guard is irrefutable (`_` or a binding), or
when its arms cover every variant of an enum, both values of a `bool`, `Ok` and `Err`, or
every value of an integer type through literals and ranges. Optional values and strings
need an irrefutable arm. Matches on tuples are not checked.

## 15.4 Dialect rules and effects (task A6)

**Constructs per dialect** (`NE0501`) follow the table in chapter 9 §9.1. In addition:
`hdl` accepts no floating-point types or literals; `list`, `map` and `text` exist only in
`host`; a `rung` contains only assignments, `set`/`reset` and function-block calls;
`move` belongs to `robot` and `cell`, `set`/`reset` coils to `plc`.

**Recursion** (`NE0302`). A function that can reach itself through calls is an error in
every dialect except `host`. This includes `core`, because `core` code must be usable in
every device dialect.

**Aliasing** (`NE0303`). In one call, a place passed `mut` SHALL NOT overlap another
argument, the receiver, or any module-level state that the callee reads or writes,
directly or through the functions it calls. Two places overlap when one is a prefix of
the other; an index overlaps every element.

**Escaping references** (`NE0305`). A slice type SHALL NOT be a return type, a field or
the type of a variable, and a slice SHALL NOT be bound with `let`.

**Bounded loops** (`NE0403`). In `core`, `mcu`, `plc`, `hdl` and `robot`, a `for` loop
SHALL iterate a range with constant limits or an array of known length, and `while` and
`loop` SHALL have `max N` with a constant `N`.

**Waiting** (`NE0404`). A task, interrupt, rule, rung, state machine, circuit or safety
block SHALL NOT contain `wait`, `until`, `move` or `->`, nor call a function that does,
nor call `delay` or `sleep`. Only a `sequence` waits.

**Deadlines on `host`** (`NE0405`). `deadline` is not allowed in `host` tasks.

## 15.5 Time, concurrency and memory (task A7)

### Activities

The scheduler releases **activities**: `task`, `on interrupt`, `fsm` with `every`, and the
rules of `safety:`. Effective priority: `prio N` (default 1) for tasks and state
machines, `1000 + N` for interrupts, `2000 + N` for safety rules. Interrupts and safety
rules therefore always preempt tasks.

Periods, offsets, deadlines, `scan`, `watchdog` and `rate` SHALL be constant durations
(`NE0407`). A period SHALL be a multiple of the base tick: 1 ms in `mcu`, the `scan`
time in `plc`, or the value given to the analysis (`NE0401`). A deadline SHALL NOT exceed
the period (`NE0406`).

### Shared resources and the Stack Resource Policy

A **resource** is a module-level `var`, `io` binding or plc variable. The set of
resources an activity touches includes those touched by the functions it calls. The
**ceiling** of a resource is the highest priority of the activities that touch it.

* `reads` and `writes` lists are optional; when present they SHALL cover every access
  (`NE0301`).
* A `var` touched by activities on different cores is `NE0304`; use a `channel` or a
  `signal`.
* A `signal` has one writer activity (`NE0306`), so that its value at each deadline is
  well defined (LET, chapter 6).

> **SRP-1 and SRP-2** hold by construction: the analysis introduces no nested
> resource acquisition, so accesses never overlap on a core and a deadlock cannot form.
> **SRP-3** is checked on the computed blocking terms: an activity is blocked by at most
> one critical section of a lower-priority activity whose resource ceiling is at least its
> own priority, never by an equal or higher priority one.

### Worst-case execution time and response time

The cost model counts abstract operations: one per statement and per expression node,
four per call of an external operation, the cost of the callee per call of a user
function, the maximum branch per `if` and `match`, and `N` times the body per loop of
bound `N`. With a CPU clock `f` and `c` cycles per operation:

```text
WCET = ceil(ops · c · 10^9 / f)   nanoseconds
```

For each periodic activity `i` on a core, with blocking `B_i` and the activities `j` of
higher priority on the same core, the worst-case response time is the least fixed point of

```text
R_i = C_i + B_i + Σ_j ceil(R_i / T_j) · C_j
```

and `R_i > D_i` is `NE0402`. Interrupts take part only when they declare
`@min_interval(T)`; otherwise `NL0403` says that they are left out. Without a CPU clock
no execution time is computed and `NL0402` reports the analysis as unavailable.

### Memory (MEM-1)

For each module: **static** bytes (module `var`s, plc variables and `io` state; signals
count twice for LET), plus the **stack** of the worst preemption chain (for each core and
priority level the largest stack of its activities, summed over the levels, because only
higher priorities can preempt), plus the **runtime** reserve. The total SHALL NOT exceed
the RAM of the target (`NE0604`). A `stack S` budget smaller than the computed need is
also `NE0604`. Sizes are packed; the backend adds alignment.

## 15.6 Safe state and verification (task A8)

**Safe state** (`NE0802`). In `mcu`, `plc` and `robot` modules, every output-capable
binding (`gpio.output`, `gpio.open_drain`, `pwm`, `dac`, plc `outputs:`) is assigned or
called in `safe_state:`. **`on fault`** SHALL NOT drive outputs (`NE0803`). Outputs of
`hdl` circuits take their safe value from the device manifest (task B7).

### Automata

A state machine becomes an automaton with its states and transitions. A sequence
becomes one automaton whose states are its steps (statements before the first step form
a `start` state, the last step leads to `done`); `parallel:` branches become automata
that start when the parallel step is entered, and the step after the block is entered
when all of them are done.

| Construct | Transition |
|---|---|
| `-> B when c` | to `B` when `c` holds |
| `-> B after D` | to `B` once `D` has elapsed in the state |
| `-> B when c or after D` | either |
| `until c` | to the next step when `c` holds |
| `until c timeout D -> X` | additionally to `X` after `D` |
| `wait D` | to the next step after `D` |
| `-> X` | to `X` |

Transitions are tried in source order and the first enabled one fires. A state that
raises `fault` is terminal, and so is a `final` state.

### Static checks

* `NE0904`: a state machine has exactly one `initial` state.
* `NE0902`: a state is unreachable from the initial state, ignoring transitions whose
  guard is `false`.
* `NE0903`: a state has no outgoing transition and is neither `final` nor a fault state,
  or a step waits on `until false`.

### Properties

`verify:` accepts three forms over state names, written `State` or `Machine.State`,
and boolean conditions:

| Property | Violated when |
|---|---|
| `never P` | some reachable moment satisfies `P` |
| `always P [implies Q]` | some reachable moment satisfies `P` and not `Q` |
| `eventually_within D after E: G` | `E` holds and `G` does not hold at any of the next `D` |

Every condition that is not an automaton state, a boolean operator or a constant is an
**atom**: an input that the environment may set to any value at every tick, independently
of the others. The checker enumerates all valuations, so a verified property holds
whatever the environment does, and it can fail because of an implausible environment. A
property that mentions no state at all (only atoms) cannot be decided and is reported
as `NL0901`.

**Time** is discrete. The tick is the greatest common divisor of all durations in the
module, coarsened to at most 4096 ticks for the longest one (`NL0901` says so when it
happens). Evaluation periods of `every` are not modeled: every automaton is evaluated at
every tick.

**Search.** Breadth-first over (states, dwell times, monitor), so the counterexample
is a shortest one. `NE0901` carries the trace; `Analysis::reports.counterexamples`
holds it with the state of every automaton and the value of every atom at every tick.
Limits: 14 atoms and the state budget of the options; beyond them the result is
`NL0901`, never a silent pass.

> **SAFE-1 and SAFE-2** concern execution and are tested on the interpreter (task A10).
> Here the static part is checked: every output has a safe value and `on fault` cannot
> change it.

## 15.7 What the analysis does not do yet

* Peripheral and pin legality, clock solving (task B1).
* Workspace limits of robots (`NE0804`) and zone locking (`NE0805`), which need
  manifests (tasks C3 and D4).
* Clock domain crossings in `hdl` (`NE0702`) and electrical compatibility (`NE0801`).
* Types of the standard library (task A12).
* Execution of programs and the properties LET-1, SAFE-1 and SAFE-2 (task A10).
