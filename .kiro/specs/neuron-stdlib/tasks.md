# Tasks: Neuron standard library, base layer (A12)

- [ ] 1. Package skeleton: `lib/neuron.toml`, layer folders, `lib/README.md`.
- [ ] 2. `core` and `units`: math helpers, bit helpers, constants, conversions, with tests.
- [ ] 3. `time`: `Ton`, `Tof`, `Tp`, edge detectors, rate limiter, with tests.
- [ ] 4. `logic`: `Sr`, `Rs`, `Ctu`, `Ctd`, `Ctud`, selectors, with tests.
- [ ] 5. `control`: `Pid` with anti-windup, `Lowpass`, `MovingAverage`, `Hysteresis`,
      `Debounce`, `Ramp`, with tests.
- [ ] 6. `test`: assertion helpers.
- [ ] 7. Differential and property tests in `crates/neuron-interp/tests/a12_stdlib.rs`.
- [ ] 8. CI: `neuron check lib` and `neuron test lib`.
- [ ] 9. Per-layer READMEs, roadmap, evidence log.
