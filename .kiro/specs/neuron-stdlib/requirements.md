# Requirements: Neuron standard library, base layer (task A12)

Source: `spec/language/12-stdlib.md` (sections 12.1, 12.2, 12.7, 12.8).

## User stories

- As a student, I want timers, counters, edge detectors and a PID controller that I
  can read in Neuron itself, so that I learn how they work.
- As a control engineer, I want the library blocks to behave like the IEC 61131-3
  function blocks I already know, so that translated programs keep their behavior.
- As a maintainer, I want every library module tested by `neuron test`, so that a
  change in the compiler or the interpreter cannot break the library unnoticed.

## Acceptance criteria (EARS)

1. WHEN `neuron check lib` runs THE SYSTEM SHALL accept every module of `lib/` with
   no error.
2. WHEN `neuron test lib` runs THE SYSTEM SHALL execute every `test` block of the
   library and every one SHALL pass.
3. THE library SHALL provide, in Neuron source, the layers `core`, `units`, `time`,
   `logic`, `control` and `test` of spec 12.8 (must have).
4. THE library SHALL contain no `io`, no `uses regs`, no `trusted` and no
   allocation (spec 12.1, 12.2).
5. WHEN a library timer or counter and the interpreter's native block receive the
   same inputs THE SYSTEM SHALL produce the same outputs at every step
   (differential test, property **STD-DIFF**).
6. FOR EVERY library controller THE SYSTEM SHALL keep its output inside its declared
   limits (property **STD-PID-1**) and SHALL stop integrating while the output
   saturates (anti-windup, **STD-PID-2**).
7. WHEN a library module imports a module of a later layer THE SYSTEM SHALL report
   `NE0106`; WHEN it forms a cycle THE SYSTEM SHALL report `NE0104`.
8. THE documentation SHALL list every `pub` item of each layer in `lib/README.md`
   (written by hand until `neuron doc` exists).

## Out of scope

`hal`, `bsp`, `proto`, `motion`, `dsp`, `drivers`, `safety` and `ui` (phase B and
later, spec 12.8 groups 2 and 3).
