# 14 · Models of computation

Neuron has one semantic Core (types, units, expressions, contracts) and several
**models of computation** (MoC). A MoC fixes *when* things happen. Each dialect
selects one (09). This chapter defines each MoC precisely enough that the
interpreter, the instruction-set simulators and the code generators can be checked
against each other.

## 14.1 Common definitions

- **Time** is a totally ordered set of instants with integer nanosecond resolution.
- A **signal** `s` is a function from instants to values. `s(t)` is the value at `t`.
- A **state** `σ` maps every `var`, `signal`, and instance field of a module to a
  value.
- An **observation** of a program is the sequence of `(t, name, value)` triples for
  every `pub` signal, every `io` output and every `fault`.
- Two executions are **equivalent** when their observations are equal after
  projecting out the instants that the MoC declares unobservable (14.9).
- **Traps** (overflow, contract violation, division by zero) end the current
  activity and raise `fault` with the matching name. A fault moves the program to
  its declared safe state (08).

## 14.2 Reactive-periodic MoC (`core`, `mcu`)

Activities: tasks, interrupts, rules, state machines.

**Release.** A periodic task with period `P` and offset `O` is released at
`t_k = O + k·P`. An event task is released at the instant its event is raised and
the earlier release has completed, otherwise the release is queued (bounded).

**Execution.** A release `r` of task `T` computes
`(σ', W) = body_T(σ, R)` where `R` is the read set of signals sampled at the release
instant (LET) and `W` is the write set.

**Visibility.** Signal writes in `W` become visible at the deadline instant
`d = release + deadline_T`. `var` writes are immediately visible, serialized by SRP
(06.4).

**Priority.** At each instant the runnable activity with the highest priority runs.
Interrupts preempt tasks. Within a priority, activities run in release order.

**Meaning.** The *logical* semantics ignores execution time: every release is
treated as instantaneous at its release instant for reads and at its deadline for
signal writes. A compiled program is *correct* when every deadline is met (06.5);
then the physical execution is equivalent to the logical one (LET-1).

> **Property MOC-R1 (determinism).** For fixed inputs (values and instants) the
> observation is unique. Nondeterminism arises only from the inputs.

## 14.3 Scan-cycle MoC (`plc`)

State: input image `I`, output image `Q`, memory `M`.

One **scan** at instants `t_n = n·S` (`S` is the declared scan time) performs, in
order:

1. `I := read_inputs(t_n)`
2. each `rung` and each `when` rule of the program unit is evaluated **in source
   order**, reading `I` and `M`, writing `M` and the pending output image `Q'`;
3. `Q := Q'`, then `write_outputs(Q)`.

Consequences, as in IEC 61131-3:

- a value written to `M` or `Q'` in a scan is visible to **later** rungs of the
  same scan and to every rung in the next scan;
- inputs do not change during a scan;
- the last write to an output in a scan wins.

> **Property MOC-P1 (scan atomicity).** No rung observes an input value that
> differs from that read at the start of its scan.

Function blocks (`class` instances such as `ton`) are evaluated at the point where
they are invoked in the rung order; their internal timers advance by `S` per scan
unless the target provides a wall clock, in which case elapsed time is used and
the difference is reported.

## 14.4 Synchronous-hardware MoC (`hdl`)

State: register values. Time is a sequence of clock edges per clock domain.

At each active edge of clock `c`:

1. every combinational expression is evaluated from the register values just
   **before** the edge and from the inputs;
2. every `reg` in domain `c` takes its next value simultaneously (all right-hand
   sides are evaluated before any register changes).

- Combinational logic has no memory; a combinational loop is `NE0502`.
- A signal read in a domain other than the one that drives it must pass a
  declared synchronizer (`NE0702`).
- All widths are explicit, and arithmetic wraps (`@overflow(wrap)` is the default).

> **Property MOC-H1 (cycle accuracy).** The simulated register values after `n`
> edges equal those of the synthesized netlist after `n` edges, for the same inputs.

## 14.5 Motion MoC (`robot`)

A robot program is a **sequence** (06.9) whose steps are **motions** and **actions**.

- A motion is `(path, limits)`: a geometric path in joint or Cartesian space and
  limits on velocity, acceleration and jerk. Its duration comes from the profile
  generator of the robot model, so it is a function of the path and the limits.
- Steps execute one after another; a step ends when its motion has settled within
  the declared tolerance (`settle` clause).
- `blend(r)` lets a step end `r` before the target and begin the next motion from
  the blend point.
- Motion is the only time-consuming construct. All other statements execute in the
  control interpolation tick (`@tick(4 ms)` by default).

> **Property MOC-M1 (limits).** For every instant of every motion, joint velocity,
> acceleration and jerk are within the limits of both the robot model and the
> motion.
>
> **Property MOC-M2 (workspace).** Every target and every interpolated sample lies
> inside the declared workspace and outside declared keep-out volumes. A violation
> detected at compile time is `NE0804`; at run time it raises `fault`.

## 14.6 Event-driven UI MoC (`ui`)

A `ui` module is a **view** over signals. Its model is a function from signal values
and input events to a tree of widgets.

- The tree is recomputed at the UI rate (default 30 Hz) or when a bound signal
  changes.
- Handlers run to completion in the order events arrive. A handler may write only
  `intent` signals; the owning module decides whether to apply the intent.
- A UI never writes a safety signal. A hardwired E-STOP stays the authority (08).

## 14.7 Host MoC (`host`)

Ordinary sequential programs with cooperative tasks. Unbounded loops are allowed
(04.3). The host MoC has no real-time guarantee. It is used for tools, tests and
orchestration, and never for code that moves hardware.

## 14.8 The cell MoC

A `cell` composes devices, each with its own MoC, over links.

**Cell clock.** A monotonic clock `T` with a resolution of 1 ns. Each device `d` has a
local clock `τ_d` with offset `o_d` and drift `δ_d`: `τ_d(T) = o_d + (1 + δ_d)·T`.

**Link.** `link a -> b via m every P within L` carries values of `a`'s signal to `b`.
For a message produced at `T`, the delivery instant is bounded by

```text
latency = serialize(m, size) + transport(m) + wait_for_receiver_phase + polling
```

where:

- `serialize(m, size)` is `size · 8 / bitrate(m)`;
- `transport(m)` is the worst-case transport delay of the medium, taken from the
  device timing profiles (cable, switch, bus arbitration);
- `wait_for_receiver_phase` is at most the receiver's period (a message waits for
  the next release);
- `polling` is the sender's period when the value is sampled, not event-driven.

The compiler requires `latency ≤ L` (`NE0701` otherwise) and proposes a faster
medium, a shorter period or an event-driven link.

**Crossing rules.**

| From | To | Mechanism |
|---|---|---|
| fast domain | slow domain | latch with acknowledge: the value is held until the receiver confirms it |
| slow domain | fast domain | value sampled at the fast domain's next edge, flagged `stale` when older than `L` |
| any | any, action taking time | `BUSY`, `DONE`, `TIMEOUT` handshake (06.9) |

**Loss of a link.** When no message arrives within the cell watchdog (default
50 ms), every device that depends on the link enters its safe state.

> **Property MOC-C1 (bounded delivery).** If the cell compiles, every link delivers
> within `L` or the watchdog fires.
>
> **Property MOC-C2 (safe on loss).** After a link loss, no actuator driven through
> that link remains energized beyond the watchdog interval plus the device's safe-state
> time.

## 14.9 Equivalence between executions

Different executions of the same program (interpreter, instruction-set simulator,
real hardware, translated program) are compared on observations.

- **Observable:** the values of `pub` signals at their deadline instants, `io`
  outputs at their commit instants, `fault`s, and the order of events.
- **Unobservable:** execution time inside a release, internal temporaries, the order
  of independent releases that do not share resources.

Two executions are **equivalent** when, for every observable name, the sequence of
`(value)` entries is equal and the instants differ by at most the declared
tolerance. The tolerance is zero for `core`, `plc` scan results and `hdl`, one base
tick for `mcu`, and the profile's settle tolerance for `robot`.

## 14.10 Refinement criterion for translation

An importer or emitter (11) produces a program `Q` from a program `P` in another
language. The translation is accepted at a given **fidelity level**:

| Level | Meaning | Check |
|---|---|---|
| `exact` | `Q` is equivalent to `P` (14.9) on all inputs | differential testing with generated and corpus inputs; formal check where the verifier can decide it |
| `refines` | every observation of `Q` is an observation that `P` permits (`P` is more permissive, for example in timing) | same tests, with timing compared as bounds |
| `approximate` | `Q` differs from `P` in documented ways | the differences are listed in the fidelity report |
| `unsupported` | no translation | reported, never silent |

The fidelity report (11.4) records the level per construct. A claim of `exact` or
`refines` without a passing differential run is a defect in the tool, not a
property of the program.

> **Property XLATE-1 (no silent change).** Every construct of `P` is either
> translated at a recorded level or reported as `unsupported`. The translation never
> drops a construct without a report entry.
