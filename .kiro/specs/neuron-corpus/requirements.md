# Requirements: validation corpus and freeze (task A13)

Source: `docs/roadmap.md` task A13, gate G1.

## Acceptance criteria (EARS)

1. THE repository SHALL contain at least 50 programs in `examples/corpus/`, with at
   least one in each of the eight dialects.
2. WHEN `neuron check` runs on a corpus package THE SYSTEM SHALL report no error.
3. WHEN `neuron fmt --check` runs on a corpus package THE SYSTEM SHALL report no file
   to reformat.
4. WHEN `neuron test` runs on a corpus package THE SYSTEM SHALL pass every `test`
   block.
5. WHEN the interpreter runs a corpus program for two simulated seconds THE SYSTEM
   SHALL raise no runtime trap (`Overflow`, `DivideByZero`, `ContractViolation`,
   `LoopBound`).
6. THE repository SHALL contain a usability review (`docs/usability-review.md`) that
   lists each point where writing the corpus failed at the first attempt.
7. WHEN the freeze is declared THE repository SHALL carry the tag `spec-v1.0-rc1`
   and the specification SHALL state its version and its open items.

## Out of scope

The final tag `spec-v1.0` needs a reviewer who did not write the specification. It is a
human action.
