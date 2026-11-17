# 17 · The reference interpreter

The reference interpreter runs a NIR package (chapter 16) in **simulated time** and
records what it does. It is the executable form of the models of computation of
chapter 14: the instruction-set simulators, the code generators and the digital twin
are checked against it (14.9). It is not fast and it does not model electronics. It
models *when things happen*.

- **Implementation.** `crates/neuron-interp`. It reads NIR only; it does not depend on
  the parser.
- **Command line.** `neuron run` and `neuron test` (17.2, 17.8).
- **Determinism.** The same package, options and scenario give the same trace, byte for
  byte (property **MOC-R1**, 14.2). There is no wall clock and no unseeded randomness.

## 17.1 What is executed

| Dialect | Executed |
|---|---|
| `core`, `mcu`, `host` | tasks, interrupts, `when` rules, state machines, sequences, `safety` |
| `plc` | scan cycle (14.3): inputs, rungs and rules in source order, outputs; function blocks |
| `robot` | sequences; a `move` takes `Options::move_ns` (a duration, not a trajectory) |
| `ui` | `on press` handlers of widgets, driven by `press <widget>` in a scenario |
| `cell` | sequences that call devices (17.7); links, zones and timing are checked by the compiler, not run |
| `hdl` | not executed; `neuron run` reports it and runs the other modules of the package |

A package runs as a whole: every module is active at once, as a set of devices.

## 17.2 Running a program

```text
neuron run [--for 2s] [--scenario file.scn] [--jitter seed] <file|dir>...
```

The output is the **trace**, one line per observation, in time order:

```text
      0.000 ms  OUT    led = true
    500.000 ms  OUT    led = false
```

| Kind | Meaning |
|---|---|
| `OUT` | an `io` output, a pwm duty, or a PLC output changed |
| `SIG` | a `signal` became visible (at its deadline, 17.4) |
| `STATE` | a state machine or a sequence entered a state or step |
| `EVENT` | an `emit`, a timeout, or a device action started or finished |
| `FAULT`, `RESET` | the safety logic (17.6) |

### Scenarios

A scenario scripts the inputs. One step per line; `#` starts a comment.

```text
at 100 ms: button = true       # an io input, a PLC input or a device field
2 s: sensor = 2048
3 s: temp = 95 degC            # a number with a unit
4 s: tick!                     # raise an event or a hardware event
5 s: press start               # press a widget
```

Names are the names of `io` bindings and PLC inputs, or dotted names of device fields
(`plc.sensor`). Inputs that were never driven read as `false` (digital) or `0` (analog).

## 17.3 Values and units

- Integers are exact and checked against the type of the expression: overflow raises
  `fault Overflow`, division by zero `fault DivideByZero` (chapter 3, 3.4). `+%` and
  `+|` wrap and saturate.
- A quantity is stored in **base units** (`degC` as kelvin, `ms` as seconds), together
  with the unit it is shown in. `1 s + 250 ms` is `1.25 s`; `degC - degC` is a
  difference. Traces show the declared unit (`62.5 degC`).
- Quantities are real numbers in the interpreter: the range check of an integer type
  with a unit (`u16 mV`) is not applied.
- `f32` results are rounded to single precision after each operation.

## 17.4 Scheduling

Time is an integer number of nanoseconds. Everything that happens at one instant
happens in this order:

1. **Signal commits**: values written by activities whose deadline is this instant.
2. **Inputs** from the scenario and completions of device actions.
3. **Events** raised from outside (`name!`, edges of inputs).
4. **Releases** of periodic activities.

The activities released at the instant then run to completion, one at a time, highest
priority first; equal priorities run in release order. Priorities are:

| Activity | Priority |
|---|---|
| `safety` | 2000000 |
| `on interrupt` | 1000000 + `prio` |
| `task` | `prio` (default 1) |
| rules, state machines, sequences, scans | 1 |
| property-style function blocks | 0 |

A body runs **instantly** in simulated time (chapter 14.2). A `task` has logical
execution time (LET, chapter 6.6): it reads every `signal` as it was at its release, and
its writes become visible at `release + deadline` (the period when there is no
`deadline`). Writes to `var` are immediate. A task without a deadline (an interrupt, an
event task) commits its signals at once.

With `--jitter <seed>` every task body starts after a random delay of up to one base
tick, with the signals and inputs it saw at release. **LET-1** says the committed
signals do not change; the test suite checks it on the examples.

Periodic activities are released at `offset + k · period`. Sequences and state machines
without a period are evaluated every `Options::base_tick_ns` (1 ms) and every 10 ms
respectively.

## 17.5 Activities

**Task, interrupt.** The body runs at each release. An interrupt is released by an edge
of an input (`button.falling`) or a scenario event (`timer2.update!`).

**Rule** (`when`). Fires on the rising edge of its condition, or once when the condition
has held for the `for` duration. In a `plc` module rules are evaluated in the scan.

**State machine.** The first evaluation enters the initial state and runs its `entry`.
Each evaluation runs `do`, then checks the transitions in source order; the first whose
`when` is true, or whose `after` has elapsed since entry, fires: `exit`, the transition
action, then the `entry` of the target. At most one fires per evaluation.

**Sequence.** A step runs its statements one after another until one has to wait.

| Statement | Behavior |
|---|---|
| `run x.m(...)` | starts an asynchronous action (17.7) and goes on |
| `until c` | waits until `c` is true |
| `until c timeout D -> S` | on timeout jumps to step `S` |
| `wait D` | waits `D` |
| `-> S` | jumps to step `S` |
| `parallel:` | runs its branches, continues when all have finished |
| `move ...` | waits `Options::move_ns` |

**PLC scan.** Each scan samples the inputs, runs every `rung` and `when` rule in source
order, advances the function blocks by the scan time, and then publishes the outputs
(14.3, MOC-P1). Outputs are published when they change; the first scan publishes the
initial image of every output.

**Function blocks.** `ton`, `tof`, `tp`, `ctu`, `ctd`, `ctud`, `r_trig`, `f_trig`, `sr`,
`rs`, `pid`, `lowpass`, `highpass`, `moving_average`, `hysteresis`, `debounce` and
`ramp` are provided by the interpreter. They are used in two styles. Call style
(`ctl.step(error)`, `filter.step(x)`) advances the block by the period of the calling
activity and returns its output. Property style (`jam.input = x` and `jam.q`) is
advanced once per scan or per base tick, so `q` reflects the input of the previous
step, as in IEC 61131-3.

## 17.6 Faults and the safe state

`fault Name`, a trap (`Overflow`, `DivideByZero`, `ContractViolation`, `StackOverflow`,
`LoopBound`) or a failed `assert` outside tests does this, immediately. `LoopBound` is the
interpreter's own limit: an activity that executes more than 5 million statements, or
builds a collection or a loop larger than 2^20 elements, is stopped.

1. records `FAULT Name`;
2. applies every `safe_state:` block of the package;
3. stops every activity except the `safety` logic;
4. runs the matching `on fault` blocks.

The program stays in the fault until a `reset when` condition of a `safety` block is
true. Then it records `RESET` and resumes.

> **Property SAFE-1.** From a `FAULT` entry until the next `RESET`, no output changes
> except by `safe_state` and `on fault`. The test suite checks it on generated
> scenarios.

## 17.7 Devices outside the package

Names that the package does not define (a device of a cell, a gripper) are *opaque*.
Reading `device.field` gives the scenario input of that name, or false. A call on an
opaque value is a no-op, except inside `run`: it starts an **action** that takes
`Options::action_ns` (default 100 ms). While it runs, `device.busy` is true; when it
finishes, `device.busy` is false and `device.done` is true. This is the
BUSY / DONE / TIMEOUT handshake of 14.8 in its simplest model. A timeout
(`until x.done timeout 3 s -> Fault`) is therefore only taken when the action takes
longer than the timeout, for example with `--action-time 5s`.

## 17.8 Tests

A `test` block runs on a fresh copy of the program. Time does not pass until the test
says so. The functions of `std.test` are legal in these blocks:

| Function | Effect |
|---|---|
| `drive(target, value)` | sets an input now (an `io` binding, a PLC input, or a name in quotes) and lets the program react |
| `advance(duration)` | lets simulated time pass |
| `elapsed()` | simulated time since the start, in seconds |
| `current_fault()` | name of the fault the program is in, or `""` |
| `state_of("Door")` | name of the current state of a state machine |

`assert cond, "message"` fails the test. `neuron test` prints `PASS` or `FAIL` for each
test with the location of the failed assertion, and exits with 1 if any fails.

```neuron
module toggle_test for mcu
io led: gpio.output @ D13
io button: gpio.input @ D2

on interrupt button.rising (prio 2):
  led.toggle()

safety:
  safe_state:
    led.write(false)

test "the button toggles the led":
  drive(button, true)
  assert led.read() == true, "on after the first press"
  drive(button, false)
  drive(button, true)
  assert led.read() == false, "off after the second press"
```

## 17.9 Limits

The interpreter is a reference for behavior in time, with these known limits, each of
which is reported rather than hidden:

- `hdl` circuits are not executed (they are checked by the compiler and, in phase B7,
  simulated by the Verilog flow).
- Robot motions have a duration but no trajectory; workspace checks are static (D4).
- Class invariants are not checked at run time.
- Sequences with parameters start once, with their parameters unset.
- Link latency and clock drift of a `cell` are analyzed (chapter 14.8), not simulated.
