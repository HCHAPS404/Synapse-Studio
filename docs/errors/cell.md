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
## Electrical
### NE0801 incompatible voltage levels without isolation or level shifting
A wired link (a medium of the `zero_hop` family) joins the output domain of the sender
to a different input domain of the receiver, for example a 3.3 V FPGA pin to a 24 V PLC
input, or a 3.3 V board to a 5 V board. The domains come from the `[io]` tables of the
two device manifests. Network and bus media do not join I/O domains and are not
checked. Use an isolating or level-shifting option of the medium (`zero_hop(gpio,
opto_24v)`), which the help spells out, or a network medium. The check is a design aid:
the real wiring must still be reviewed against the datasheets of the parts.
## Tags
### NE0703 tag mapped to incompatible types or units on two devices
`tag t: T @ d1.addr, d2.name` maps one value onto several devices (spec 11.2). The code
covers every way the mapping can be wrong. An IEC direct address (`plc.%MD20`) must be
well formed, start with an address family of the device manifest (`[io]` prefixes; the
area letter is enough, so `%MD20` fits a family `%M` or `%MW`), and have the width of the
tag's type: a `bool` needs a bit address (`%MX`, `%IX`, `%QX`, or none as in `%I0.0`),
`u8`/`i8` `%MB`, `u16`/`i16` `%MW`, `u32`/`i32`/`f32` `%MD`, `u64`/`i64`/`f64` `%ML`. A
symbol location (`ctl.speed`) must have the same type and unit as the `signal`, `var`,
PLC variable or `io` binding of that name in a module placed on that device; a symbol no
placed module declares is not checked. A tag also has one writer: a location is a write
location when the device's program produces the value (a placed module assigns the
symbol, the address is an output `%Q...`, or it is a memory address bound by `@` to a PLC
variable that the module assigns); write locations on two devices are two writers, and
the message lists them. Keep one write location and make the other devices read it, or fix
the type, unit or address the message names. Locations that are not an address or a
plain symbol (`plc.db(1).x0`, `opcua(..)`) are not checked yet. A location on a name that
is not a device of the cell is `NE0607`.
## Zones
### NE0805 zone entered without a lock, or a lock the planner cannot build
`zone z: box(x: .., y: .., z: ..)` names a volume and `mutex z between a, b.x` lets the
listed devices enter it one at a time. The code covers every way that arrangement can
fail. A robot `move` whose constant target lies in the zone (in the base frame of the
robot, which is taken as the cell frame; the flange target, tool offsets not applied)
on a device that is not a party of a `mutex` of that zone enters it without a lock: add
the device to the mutex, or move the point. A `mutex` that names a zone the cell does
not declare, a `zone` that is not a constant `box` with units, a party that is not a
device of the cell, a party output (`plc.press_cylinder`) that no module placed on the
device declares, a party that runs a dialect that cannot take part in a lock (`hdl`),
no party that can run the arbiter (`plc`, `mcu` or `host`), and a party with no medium
to the owner of the lock are the other causes. The lock is a four-phase request/grant
handshake generated as Neuron source on each party (`links/lock_*.neuron` in the device
build); the owner is a `host` that is a party, else the first party that can arbitrate.
The planner lists the entries (motions and party outputs) in `zones.json`, but it does
not rewrite your sequences: calling acquire before an entry and release after it is part
of the contract of the program. Targets that are not constant Cartesian points (joint
targets, computed values) cannot be placed in the zone statically and are counted in
`unchecked_targets`.
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
