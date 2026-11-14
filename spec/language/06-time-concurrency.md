# 06 · Time and concurrency

## 6.1 Time types

- `duration` literals: `10 ns`, `5 us`, `500 ms`, `2 s`, `1 min`.
- `instant` comes from `now()`; `instant - instant` is a `duration`.
- Every program has a **base tick** given by its target and dialect (default 1 ms
  on `mcu`, the scan period on `plc`, the clock period on `hdl`). Periods SHALL be
  multiples of the base tick (`NE0401`) unless the target provides a finer timer.

## 6.2 Tasks

```neuron
task control every 10 ms offset 2 ms (prio 2, deadline 4 ms):
  heater.duty = pid.step(target - temp)

task report on report_requested (prio 1):
  uart.print("T={temp:.1}")
```

| Clause | Meaning | Default |
|---|---|---|
| `every P` | periodic release every `P` | — |
| `on E` | released by event `E` (`emit E`) | — |
| `offset O` | phase of the first release | `0` |
| `prio N` | static priority, higher runs first | `1` |
| `deadline D` | completion deadline relative to release | `P` |
| `core C` | core assignment on multicore targets | compiler chooses |
| `stack S` | stack budget | computed |

Tasks are **run-to-completion**: a task body never blocks. Waiting is expressed with
`fsm`, `sequence` or events, never with delay loops.

## 6.3 Interrupts

```neuron
on interrupt btn.falling (prio 3):
  presses += 1

on interrupt timer2.update (prio 5):
  encoder.sample()
```

- The source names a hardware event of an `io` binding (07).
- Interrupt priorities are above all task priorities on the same core. If the target
  cannot nest interrupts with priorities, nesting is disabled and `NL0601` reports
  the change.
- Interrupt bodies obey the same rules as task bodies.

## 6.4 Scheduling and the Stack Resource Policy

On each core the runtime uses fixed-priority preemptive scheduling. Every shared
resource `r` gets a **ceiling** `ceil(r)` = the highest priority of the activities
that access it. While an activity accesses `r`, the runtime raises the current
priority to `ceil(r)` (SRP / immediate priority ceiling).

> **Property SRP-1 (no data races).** No two activities ever access the same
> resource concurrently.
>
> **Property SRP-2 (no deadlock).** On a single core, an accepted program cannot
> deadlock.
>
> **Property SRP-3 (bounded blocking).** Each activity is blocked at most once per
> release, for at most the longest critical section of a lower-priority activity
> with a ceiling at or above its priority.

Cross-core communication uses channels and signals, never shared `var`s (`NE0304`).

## 6.5 Schedulability and WCET

For targets with a timing model, the compiler estimates the worst-case execution
time (WCET) of every task and interrupt, then runs response-time analysis with the
blocking terms of SRP. If a task can miss its deadline, compilation fails with
`NE0402` and a report (WCET per task, utilization, the critical chain). Without a
timing model, the analysis is reported as unavailable (`NL0402`) and runtime deadline
monitoring is enabled instead.

## 6.6 Signals and logical execution time

A `signal` is a value with **logical execution time** (LET) semantics:

- a task reads every signal it uses **as of its release instant**;
- the values it writes become visible **at its deadline** (end of the logical
  execution window);
- therefore results do not depend on execution jitter, and the same program behaves
  identically in the digital twin and on hardware.

`var` is plain shared state (immediate visibility, protected by SRP); `signal` is
time-consistent and is what the cell level, the tag dictionary, the twin and the UI
observe.

> **Property LET-1.** For any two executions with the same inputs at the same release
> instants, every signal has the same value at every deadline instant, independent
> of execution times (provided all deadlines are met).

## 6.7 Rules (`when`)

```neuron
when tank.level > 95 % for 2 s:
  emit overflow_warning

when start.rising:
  fsm_main.request(Run)
```

A `when` rule fires on the **rising edge** of its condition (or when the condition
has been continuously true for the `for` duration). Rules are evaluated by an
implicit task at the module rate (`@rate(10 ms)` by default; inside `safety`, at the
safety rate, see 08). Edge helpers: `.rising`, `.falling`, `.changed` on `bool`
values and signals.

## 6.8 State machines

```neuron
fsm Door every 10 ms:
  initial state Closed:
    entry: lock.engage()
    -> Opening when open_cmd
  state Opening:
    entry: motor.forward()
    exit: motor.stop()
    -> Open when end_open
    -> Fault after 5 s
  state Open:
    -> Closing when close_cmd or after 30 s
  state Closing:
    entry: motor.reverse()
    -> Closed when end_closed
    -> Opening when obstacle
  state Fault:
    entry: fault DoorJam
```

- Evaluated every period (`every`) or on an event (`on`). Exactly one state is
  active; transitions are checked in source order; at most one fires per evaluation.
- `after D` is relative to entry into the current state.
- `do:` runs on every evaluation while in the state.
- The verifier checks reachability, absence of deadlock (a state with no exit that
  is not declared `final`) and user properties (08).

## 6.9 Sequences (GRAFCET / SFC)

```neuron
sequence Cycle on plc:
  initial step Idle:
    until start and part_present
  step Clamp:
    run clamp.close()
    until clamp.closed timeout 2 s -> Fault
  parallel:
    branch drill:
      step Drill:
        run drill.cycle()
        until drill.done
    branch blow:
      step Blow:
        valve.open()
        wait 500 ms
        valve.close()
  step Release:
    run clamp.open()
    until clamp.opened
    -> Idle
  step Fault:
    fault CycleFault
```

- Steps execute in order; `-> Name` jumps. `until cond` is a transition condition.
- `parallel:` starts all branches; the step after the parallel block activates when
  every branch has finished (synchronized convergence, as in GRAFCET).
- `run action` starts an **asynchronous action** and returns a handle with flags
  `busy`, `done`, `failed` and `timeout`; `until x.done` waits on it.
- `wait D` is legal only inside sequences; it is a timed transition, not a blocking
  call.
- A sequence body MAY be a plain list of statements (robot programs): each statement
  that takes time is an implicit step.

## 6.10 Events

`emit E` raises event `E` (optionally with a payload declared as
`event E: Payload`). Tasks with `on E`, rules and state machines can react. Events
are queued per receiver in a bounded channel; overflow is counted and reported.

## 6.11 The Standard Time Model at cell level

The `cell` dialect extends these semantics across devices:

- a **cell clock** with per-device domains (ns for `hdl`, µs/ms for `mcu`, the scan
  for `plc`, motion time for `robot`);
- `link a -> b via medium every P within L`: the compiler synthesizes the transport,
  computes the worst-case latency of the chain
  (serialization + transport + receiver phase + polling) from device timing profiles,
  and fails with `NE0701` if `L` cannot be met, proposing alternatives;
- fast-to-slow crossings get **Latch + ACK** automatically; asynchronous actions get
  **BUSY / DONE / TIMEOUT**;
- `event e: timestamp from device` attaches the source timestamp so receivers can
  compensate latency (e.g. conveyor tracking);
- clock synchronization uses PTP where the hardware supports it, otherwise offset and
  drift estimation over Neuron Wire, with the achieved accuracy reported;
- a cell watchdog (default 50 ms) puts every device in its safe state when a link is
  lost.
