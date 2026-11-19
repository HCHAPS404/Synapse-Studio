# Design: translation between languages (Phase C)

## Crates

| Crate | Content | Depends on |
|---|---|---|
| `neuron-emit` | C1 framework (`Emitter`, `Package`, `Fidelity`, `SourceMap`, `Policy`, line writer) and the emitters: `iec` (ST, SCL, PLCopen XML), `spel`, `python`, `kernel` (C6 tables and the ST, C and SPEL+ kernels) | `neuron-nir`, `serde`, `serde_json` |
| `neuron-st` | IEC 61131-3 ST subset: lexer, parser, AST and a virtual PLC that scans a program in simulated time | none |
| `neuron-spel` | SPEL+-style subset: parser and a virtual robot controller | none |
| `neuron-import` | C5 importers to Neuron source: ST, PLCopen XML, SPEL+, C subset, Verilog subset | `neuron-st`, `neuron-spel`, `roxmltree` |

Emitters read NIR only, so the twin and the web IDE can call them from WebAssembly.
Importers write Neuron *source*, which the normal analyzer turns into NIR: the output is
readable, diffable and checked by the same rules as hand-written code.

## C1 framework

- `Emitter` has `name()`, `manifest()` (languages, dialects, constructs) and
  `emit(&Nir, &EmitOptions) -> Result<Package, Vec<Diag>>`.
- `Package` holds files (`BTreeMap<String, String>`, sorted), `wiring` (Markdown),
  `fidelity` and one `SourceMap` per generated file. `Package::write(dir)` writes
  `fidelity.json` and `<file>.map.json` next to the files.
- `Fidelity` follows 11.4: `tool`, `version`, `target`, `summary` (counts per class)
  and `entries` (`file`, `span`, `class`, `construct`, `note`). Exact constructs are
  counted in the summary; anything not exact has an entry.
- `Policy { worst: Class }`: `check` returns `NE1003` for every entry worse than
  `worst`. The CLI writes nothing when the policy fails.
- `Writer` tracks line and column, so each emitted statement records a mapping
  `gen [l0, c0, l1, c1] -> src {file, span}` with the view name (`rung:run`, `step:Fill`).
- All JSON is pretty-printed with a final newline; maps and summaries use ordered keys.

## C2 IEC 61131-3

### Mapping of a `plc` module (ST, generic profile)

| Neuron | ST |
|---|---|
| `scan 10 ms` | task configuration comment and `CONFIGURATION` with `TASK ... (INTERVAL := T#10MS)` |
| `inputs:` `x: bool @ %I0.0` | `VAR x AT %IX0.0 : BOOL; END_VAR` (bit addresses get the `X` size prefix) |
| `memory:` `retain` | `VAR RETAIN` |
| `let t = ton(preset: 3 s)` | `t : TON;` and `t(PT := T#3S);` at the end of the scan |
| `t.input = e` / `t.q` | `t.IN := e;` / `t.Q` (field names: IN, Q, ET, PT; CU, CD, R, LD, PV, CV, QU, QD; CLK; S1, R, S, R1, Q1) |
| `x = e` | `x := e;` |
| `set x when c` / `reset x when c` | `IF c THEN x := TRUE; END_IF;` / `... FALSE` |
| `x += 1 when c` | `IF c THEN x := x + 1; END_IF;` |
| `h.step(v)` (hysteresis, debounce, ramp, filters, pid) | a helper `FUNCTION_BLOCK NEURON_HYSTERESIS` written into the output, called before the statement that uses it |
| `sequence` | a CASE machine over the wait points (below) |
| `fsm` | a CASE machine over the states with entry, exit and `do` |
| `when c for d:` | `TON` on `c`, fires once per rising condition |
| `safety: safe_state` | at the top of the scan: `IF neuron_fault THEN <safe state>; RETURN; END_IF;` |
| `fault F` | `neuron_fault := TRUE; neuron_fault_code := <n>;` with the code table in the wiring report |
| `reset when c` | `IF neuron_fault AND c THEN neuron_fault := FALSE; END_IF;` |

Order inside one scan matches 14.3 and the interpreter: inputs are read, rungs and
rules run in source order, then every timer, counter, edge detector and latch is called
once, then outputs are written.

### Timers and the fidelity of time

The interpreter advances a timer by one scan per evaluation, starting with the first
scan in which its input is true. A PLC timer measures wall-clock time from the scan in
which the input rose, so its `Q` comes one scan later. The emitter reports every timer
as `equivalent` with "Q may rise one scan later than in the reference". The virtual PLC
has two clock modes: `ScanCounted` (Neuron's rule, used for the exact test) and
`WallClock` (IEC behavior, used for the bounded test).

### Sequences as CASE machines

The flattened sequence (the interpreter's op list) is split at its wait points
(`until`, `wait`). Each wait point is a state. Entering a state runs the straight-line
statements that follow the previous wait point. `until c timeout d -> T` uses a TON on
the state; `-> Step` continues at the step. A loop without a wait point is `NE0610`.
Statements of a sequence run once per scan instead of once per base tick, and a timed
state starts on the next scan, so the fidelity entry gives a bound of two scans.

### Profiles

- `generic`: IEC 61131-3 third edition ST, `PROGRAM` plus `CONFIGURATION`.
- `yaskawa`: the same ST with the global I/O in a separate `GlobalVars.st` and no
  `CONFIGURATION` (the task is configured in the project). Differences beyond this are
  confirmed in session I5 and recorded in `docs/translation.md`.
- SCL (Siemens-style): a `FUNCTION_BLOCK` with `VAR_INPUT`, `VAR_OUTPUT`, `VAR`
  (multi-instance `TON_TIME` and similar), `#` local prefix, an `ORGANIZATION_BLOCK`
  `"Main"` that calls the instance with the PLC tags, and `PLC_tags.csv` (name, data
  type, logical address).
- PLCopen XML (TC6 2.01 namespace): one `pou` of type `program`, the interface, and a
  body in LD when every rung is a network of contacts, coils, set and reset coils and
  function-block boxes, otherwise in ST; sequences as an SFC body (steps, transitions
  with ST conditions, action blocks with ST). Element positions are on a grid.

## The virtual PLC (`neuron-st`)

Parser for the ST the emitter writes and for common hand-written ST: `PROGRAM`,
`FUNCTION_BLOCK`, `VAR` sections with `AT`, `RETAIN` and initial values, elementary types,
assignments, `IF/ELSIF/ELSE`, `CASE`, `FOR`, `WHILE`, `REPEAT`, `EXIT`, `RETURN`, FB
calls with named arguments, expressions with IEC precedence, `T#` literals. The VM runs
one scan per period: latch the inputs, execute, publish the outputs, record each output
change with its instant. Standard blocks: TON, TOF, TP, CTU, CTD, CTUD, R_TRIG, F_TRIG,
SR, RS. User function blocks run with their own instance state.

## C3 robot

| Neuron | SPEL+-style |
|---|---|
| `point p = joints(...) deg` | `P<n> = JA(j1, ..., j6)` with a label |
| `point p = f.xyz(x, y, z) mm` | `P<n> = XY(x, y, z, 0, 0, 0) /<local>` |
| `frame f = frame(origin, rot)` | `Local <k>, XY(ox, oy, oz, rz, ry, rx)` |
| `tool t = tool(offset, payload)` | `TLSet <k>, XY(...)`, `Weight <payload>` |
| `move ptp to p speed s %` | `Speed s` then `Go P<n>` |
| `move linear to p speed v mm/s` | `SpeedS v` then `Move P<n>` |
| `move arc to b via m` | `Arc P<m>, P<b>` |
| `blend r mm` | `CP On` for that motion, recorded as `approximate` |
| `p.above(d mm)` | `P<n> +Z(d)` |
| `io.write(true)` / `din` read | `On <bit>` / `Off <bit>` / `Sw(<bit>)` |
| `wait d` | `Wait d` (seconds) |
| `until c timeout d -> S` | `Wait <cond>, d` then `If TW Then GoTo S` |
| `for r in 0..n` | `For r = 0 To n - 1 ... Next r` |
| `fault F` | `Error <8000 + n>` with the table in the wiring report |

The robot manifest (`devices/robots/*.toml`, generic models) gives joint limits, reach,
and a Z range. Constant targets are checked at emission (`NE0804`); a target inside a
loop with constant bounds is checked for every iteration; any other computed target
gets a call to `NeuronCheck` before the motion, which raises the workspace error.

The slave (`NeuronSlave.prg`) is a TCP command interpreter: `GO n`, `MOVE n`, `OUT b v`,
`IN b`, `SPEED v`, `HOME`, each answered with `OK`, `ERR <code>` or the value; motion
commands check the point table limits first.

The virtual controller (`neuron-spel`) runs the subset with a fixed time per motion
(the interpreter's `move_ns`) and records motions and outputs.

## C4 Python

One file per module plus `neuron_rt.py` (event loop in simulated or wall time, signals
with logical execution time, periodic tasks, faults and the safe state, `drive`,
`advance`, `elapsed`). `python app.py --test` runs the test blocks in simulated time;
`--trace 3s` prints the trace in the interpreter's format; `--ros2` (optional) wraps
the program in an rclpy node that publishes the signals.

## C5 importers

- ST: `PROGRAM` variables with `AT %I/%Q/%M` become `inputs`, `outputs`, `memory`;
  standard block instances become `let x = ton(preset: ...)`; each assignment becomes
  a rung statement; `IF c THEN x := TRUE; END_IF` becomes `set x when c`; `IF c THEN
  x := e; END_IF` becomes `x = e when c`; the end-of-scan block calls become field
  assignments. Anything else gets `NE1002` and a fidelity entry.
- PLCopen XML: the LD graph is walked from each coil back to the left rail; series
  contacts become `and`, parallel branches `or`, negated contacts `not`; SFC steps and
  transitions become a `sequence`.
- SPEL+: the subset the emitter writes, plus hand-written programs with the same
  statements.
- C subset: functions over integers and booleans with `if`, `while` (with a bound
  comment `/* max N */` or a constant `for`), `return`; becomes `core` functions.
- Verilog subset: one module with `input`, `output reg`, `always @(posedge clk)` with
  non-blocking assignments and `assign`; becomes an `hdl` circuit.

Round-trip tests: corpus → emit → import → analyze → run, compared with the original.

## C6 micro-kernel

A state table has states, transitions (guard as a small postfix program over inputs,
variables and timers), and actions (write output, write variable, start or reset the
state timer). `.nst.json` is the readable form; the word encoding (`u16` words with a
header, version and checksum) is what the daemon downloads. Limits: 64 states, 128
transitions, 256 program words, 32 variables, otherwise `NE1004`. Kernels: a Rust
reference executor, an ST function block `NEURON_KERNEL` over an `ARRAY OF INT`, a C99
file `nrt_kernel.c`, and a SPEL+ function.
