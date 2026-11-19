# Tasks: Neuron standard library, base layer (A12)

- [x] 1. Package skeleton: `lib/neuron.toml`, layer folders, `lib/README.md`.
- [x] 2. `core` and `units`: math helpers (`lerp`, `map_range`, `abs_diff`, `between`),
      bit helpers (`bit_get`, `bit_set`, `bit_clear`, `popcount`), constants, with tests.
- [x] 3. `time`: `Ton`, `Tof`, `Tp`, `Stopwatch`, `Every`, with tests. Edge detectors
      live in `logic` (`RTrig`, `FTrig`). A rate limiter is not written; `control.Ramp`
      covers the rate-limited setpoint case.
- [x] 4. `logic`: `Sr`, `Rs`, `Ctu`, `Ctd`, `Ctud`, `RTrig`, `FTrig`, with tests. Selectors
      (`sel`, `mux`, `limit`) are not written; follow-up with the IEC emitters (C2).
- [x] 5. `control`: `Pid` with anti-windup, `Lowpass`, `Highpass`, `MovingAverage`,
      `Hysteresis`, `Debounce`, `Ramp`, with tests.
- [x] 6. `test`: assertion helpers (`close`).
- [x] 7. Differential and property tests in `crates/neuron-interp/tests/a12_stdlib.rs`.
- [x] 8. CI: `neuron check lib`, `neuron fmt --check lib` and `neuron test lib`.
- [x] 9. Roadmap and evidence log. Per-layer READMEs are folded into `lib/README.md`.

Follow-ups: selectors and a rate limiter (with C2), the `hal`, `proto`, `motion`,
`drivers` and `safety` layers.
