# 12 · Standard library

The standard library (`lib/`, imported as `std`) is written in Neuron wherever the
language can express it. Only the thin layer that touches silicon (register access,
startup, context switch) is `trusted` (07). This chapter fixes its structure, its
design rules and the quality bar. It does not list every function; the catalogue in
`lib/<layer>/README.md` is generated from doc comments.

## 12.1 Layers and dependency rule

Layers form a strict stack. A module may import only from its own layer or from
layers listed before it.

| # | Layer | Module prefix | Content | Dialects |
|---|---|---|---|---|
| 1 | core | `std.core` | `Option`, `Result`, ranges, math, bit operations, `Ord`, arrays and slices helpers | all |
| 2 | units | `std.units` | the unit table, conversions, constants (`pi`, `g0`) | all |
| 3 | time | `std.time` | `duration`, `instant`, timers (`ton`, `tof`, `tp`), edge detection, rate limiting | all |
| 4 | logic | `std.logic` | latches, counters, edge triggers, debounce, selectors (IEC 61131 function-block equivalents) | all |
| 5 | hal | `std.hal` | peripheral interfaces: `gpio`, `adc`, `pwm`, `uart`, `spi`, `i2c`, `can`, `timer` | `mcu`, `plc`, `robot` |
| 6 | control | `std.control` | `Pid`, filters, ramp generators, hysteresis, observers | all |
| 7 | motion | `std.motion` | trapezoidal and S-curve profiles, stepper drivers, kinematics, interpolation | `mcu`, `robot`, `cell` |
| 8 | proto | `std.proto` | Modbus RTU/TCP, CAN framing, SLIP, COBS, CRC family, Neuron Wire (11) | `mcu`, `plc`, `host` |
| 9 | dsp | `std.dsp` | FIR/IIR, FFT, windowing, fixed-point helpers | all |
| 10 | drivers | `std.drivers` | sensors, displays, motor drivers, memories written against `std.hal` | `mcu`, `plc`, `robot` |
| 11 | safety | `std.safety` | two-channel evaluation, E-STOP and guard-door patterns, watchdog helpers, diagnostics | all |
| 12 | diag | `std.diag` | logging, counters, trace points, health reports | all |
| 13 | test | `std.test` | assertions, fakes for `std.hal`, property generators | `test` blocks only |
| 14 | bsp | `std.bsp.<board>` | board packages generated from `targets/boards/` (07) | `mcu` |

Rules enforced by `neuron check` for files under `lib/`:

- No import goes upward in the table (`NE0104` when it forms a cycle, `NE0106` when
  it breaks the layering).
- Layers 1 to 4 and 6 to 9 contain no `io`, no `uses regs` and no `trusted`. They
  are portable to every dialect they list.
- Only `std.hal` implementations in `std.bsp.*` and the runtime crates may contain
  `trusted` code, and every such block appears in the audit report (08).

## 12.2 Design rules

1. **Portable by construction.** A library function compiles for every target that
   its layer lists. Where a function needs a feature (an FPU, a second UART) it uses
   `@when` and documents the fallback.
2. **Units in every public signature.** A `pub` function that handles a physical
   quantity declares it (`f32 degC`, `u16 mV`). `NL0301` flags a bare `f32` that
   carries a quantity by name or doc comment.
3. **No hidden allocation, no hidden blocking.** Nothing in `lib/` allocates outside
   `pool<T, N>` and nothing waits without a `max` bound (05, 06).
4. **State is explicit.** Anything with memory is a `class` that the caller owns.
   There is no library-global state.
5. **Contracts document behavior.** Public functions state `requires` and `ensures`
   where they are checkable. The contracts are the primary documentation of limits.
6. **Static dispatch.** Drivers take an interface, never a runtime pointer. The
   compiler monomorphizes, so a driver costs nothing when it is not used.
7. **One way to do it.** When two designs are possible the library offers one,
   and the other lives in a separate package.

## 12.3 The HAL interfaces

`std.hal` defines the interfaces that `io` bindings (07) return. Drivers and user
code are written against them. A board package maps them to registers.

```neuron
module std.hal.gpio for mcu

pub enum Pull:
  None
  Up
  Down

pub interface Input:
  fn read(mut self) -> bool

pub interface Output:
  fn write(mut self, level: bool)
  fn toggle(mut self)

pub interface Pwm:
  fn set_duty(mut self, duty: f32 %)
  fn set_frequency(mut self, freq: f32 Hz) -> Result<f32 Hz, ConfigError>
```

- A HAL interface contains only operations that cannot block. Operations that take
  time return a handle, as described in 07.2.
- Every interface has a **fake** in `std.test` that records calls and replays
  scripted inputs. The same fakes back the simulators in the digital twin, so a
  driver is tested once and runs unchanged on virtual and real devices.
- Interfaces are versioned with the library. A new method is a minor version when
  it has a default body, and a major version otherwise.

## 12.4 A library class in full

The conventions above, applied to a controller:

```neuron
module std.control.pid for mcu

pub class Pid:
  let kp: f32
  let ki: f32 Hz
  let kd: f32 s
  let out_min: f32 %
  let out_max: f32 %
  var integral: f32 = 0.0
  var previous: f32 = 0.0

  pub fn update(mut self, error: f32, dt: f32 s) -> f32 %:
    requires dt > 0.0 s
    let p = self.kp * error
    let d = self.kd * (error - self.previous) / dt
    self.previous = error
    let raw = p + self.integral + d
    let out = clamp(raw, self.out_min, self.out_max)
    if out == raw:
      self.integral += self.ki * error * dt
    return out
```

The class has no allocation, no I/O and no time source. It receives `dt` from the
caller, which makes it deterministic and testable on the host. The anti-windup rule
(stop integrating while the output saturates) is part of the contract, stated in the
doc comment and covered by a property test (12.7).

## 12.5 Display stack

Displays are the most common source of memory problems on small targets, so the
design is explicit about RAM.

| Level | Module | Responsibility |
|---|---|---|
| Bus | `std.hal.spi`, `std.hal.i2c` | transfers |
| Panel driver | `std.drivers.display.<chip>` | init sequence, windowing, pixel push |
| Surface | `std.ui.surface` | pixel format, clipping, dirty rectangles |
| Widgets | `std.ui` | labels, bars, gauges, trend plots, buttons, menus |

RAM budget for a full frame buffer (bytes = width × height × bits per pixel / 8):

| Panel | Pixels | 1 bpp | 16 bpp |
|---|---|---|---|
| 128 × 64 monochrome | 8 192 | 1 KiB | 16 KiB |
| 240 × 240 color | 57 600 | 7 KiB | 112.5 KiB |
| 320 × 240 color | 76 800 | 9.4 KiB | 150 KiB |

The surface therefore supports three strategies, chosen per target by the compiler
from the RAM report (05):

1. **Full buffer** when the budget allows (for example 150 KiB on a part with
   512 KiB of RAM).
2. **Banded rendering**: the screen is drawn in horizontal bands of `H` lines and
   each band is pushed before the next is rendered. RAM is `width × H × bpp / 8`.
3. **Direct drawing** for text and rectangles, with no buffer, when the panel
   supports windowed writes.

`MEM-1` (05) applies: when no strategy fits, the build fails with the numbers
(`NE0604`).

## 12.6 Versioning and distribution

- Library packages follow semantic versioning. The `edition` of the language is
  independent of the package version.
- `neuron.lock` pins exact versions and content hashes. A build with a stale lock
  fails with `NE0107`.
- A published package carries its **fidelity matrix**: for each dialect and target
  family, one of `tested on hardware`, `tested in simulation`, `compiles only`.
  The matrix is shown by `neuron doc` and in the package browser of the Studio.
- Deprecation takes two minor versions: `@deprecated("use x")` warns, then errors.

## 12.7 Quality bar

A module enters `lib/` only when it has:

| Item | Requirement |
|---|---|
| Doc comments | every `pub` item, with a runnable example |
| Contracts | `requires`/`ensures` for each checkable limit |
| Unit tests | `test "name":` blocks run by `neuron test` |
| Property tests | for every invariant (for example `Pid` output stays in `out_min..out_max`; a CRC of data plus its CRC is zero) |
| Conformance vector | a golden input/output pair under `conformance/` when the behavior must match another implementation (CRC, Modbus framing, FFT) |
| WCET figure | published per target family for each function used in tasks |
| No `trusted` | unless in a `bsp` module, with a written justification |

Doc examples are compiled and run in CI, so they cannot rot.

## 12.8 Scope for the 2026-12-03 submission

The full catalogue grows after the hackathon. For the submission the library
contains the following, in priority order:

1. **Must have:** `core`, `units`, `time` (timers and edges), `logic`, `hal` (`gpio`,
   `adc`, `pwm`, `uart`, `timer`), `control` (`Pid`, `lowpass`, `ramp`, `hysteresis`),
   `diag`, `test`, and `bsp` for the boards in the lab (Arduino UNO R3, Mega,
   ESP32-S3, Raspberry Pi Pico, Pico 2 W, STM32 NUCLEO-N657X0-Q).
2. **Should have:** `hal` (`i2c`, `spi`, `can`), `proto` (CRC, Modbus RTU, COBS),
   `safety` (two-channel, E-STOP pattern, watchdog), `motion` (trapezoidal profile,
   stepper).
3. **Could have:** `dsp` (FIR, IIR), `ui` widgets and one display driver.

Anything not in the first two groups is documented as planned and not claimed in
the demo.
