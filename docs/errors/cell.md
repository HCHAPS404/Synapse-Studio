# Cell errors (Phase D)
Source of truth for the catalogue: `spec/language/10-diagnostics.md`. The design is in
`.kiro/specs/neuron-cell/`; the user guide is `docs/cells.md`. `neuron explain NE0704`
prints the matching section. These checks run on a `cell` file and the modules of its
package, after the semantic analysis.
## Links
### NE0701 link latency budget cannot be met
The worst case of `link a.x -> b.y via m [every P] within L` is above `L`. The worst
case (spec 14.8) is serialization (frame bytes over the bit rate) plus transport (the
sender's output time, the medium's worst transport and the receiver's input time, each
with its jitter) plus the receiver phase (one release of the receiver's shortest scan or
task) plus polling (the period after `every`, the sender's scan without it, zero for an
event-driven medium without `every`). The message gives each term and the trust level of
the numbers: `generic` numbers are placeholders until lab session I1 measures them. The
help lists the changes that meet `L`, fastest first: a shorter `every`, another medium
both devices speak, or the event-driven form of a medium. When none does, raise `L` or
put both ends of the logic on one device.
## Placement
### NE0704 module or sequence placed on a device that cannot run it
`place m on d` names a device whose manifest does not run the dialect of `m`, or `m`
is written for a board (`module m for mcu on b`) and `d` is a different board. The help
says which dialects the device runs. The same code covers a `place` or a
`sequence ... on d` that names a device the cell does not declare, a module placed
twice, and a cell sequence on a device that runs no sequential dialect (an FPGA). Place
the module on a device that runs its dialect, or change the device type in `device`.
### NE0705 module or sequence fits no device of the cell
A module that no cell places is assigned automatically to a device that runs its
dialect; when none does, this error lists every device and what it lacks. Modules of
the `core` dialect (library code) and modules written for a board that is not in the
cell are not candidates. A cell `sequence` without `on` runs on the first PLC, else the
first host; with neither, it is this error. Add a device of the right class, place the
module explicitly, or move it to another package.
