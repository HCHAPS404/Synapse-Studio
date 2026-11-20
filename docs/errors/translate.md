# Translation errors (Phase C)

Source of truth for the catalogue: `spec/language/10-diagnostics.md` (`NE0804` is the
robot check of the `spel` emitter, documented below). The fidelity report
and the emitter and importer contracts are in `spec/language/11-interchange.md`; what each
emitter translates is in `docs/translation.md`. A construct that an emitter cannot
translate is `NE0610` (see `docs/errors/targets.md`).

### NE1001 source text of another language cannot be parsed
An importer (`neuron import st`, `plcopen`, `spel`, `c`, `verilog`) could not read its
input. The message gives the position and what was expected. The importers read the
subset listed in `docs/translation.md`; vendor project files are not read, only the source
or the exported XML.

### NE1002 construct of the source language has no Neuron equivalent
The input is valid in its own language, but this construct cannot be represented in
Neuron yet (for example a `CASE` on a step variable in ST, a pointer in C, or a latch
inferred by a Verilog `always` block). The construct is listed in `fidelity.json` as
`unsupported` and nothing is imported for it; write it in Neuron by hand.

### NE1003 translation is less faithful than the project policy allows
The project accepts translations up to a class (`exact`, `equivalent`, `approximate`,
`modeled`), set with `--policy`. An entry of the fidelity report is worse than that, so
nothing is written. The help repeats the note of the entry, which says what differs and by
how much. Change the program, accept the class explicitly, or translate that part by hand.

### NE0804 robot target outside the workspace or the joint limits
`neuron emit spel` checks every target against the robot manifest (`--robot`, default
`generic_6axis`; files in `devices/robots/*.toml`): each joint inside its limits (and the
right number of joints), the flange inside the reach measured from the shoulder point, and
its height inside the z range, after applying the frame (and `above`). A target whose
components are constants is checked at emission. A target inside a `for` with constant
bounds is checked for every iteration. Any other computed target is not an error: the
emitted program calls `NeuronCheck` before the motion, which raises error 8100 at run time
(spec 09 §9 and 14). The help names the limit that is exceeded. Move the point, change
the frame, or choose another manifest. The numbers of the generic manifests describe no
vendor's product; copy and edit one for your robot.

### NE1004 state table beyond the limits of the micro-kernel
A state machine or sequence compiled for the micro-kernel needs more than 64 states, 128
transitions, 256 program words or 32 variables. Split it into smaller machines, or emit it
as regular code for the device instead of a state table.
