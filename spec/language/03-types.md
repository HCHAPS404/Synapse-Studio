# 03 · Types and units

## 3.1 Scalar types

| Type | Meaning | Notes |
|---|---|---|
| `bool` | `true` / `false` | 1 bit in `hdl`, 1 byte elsewhere |
| `u8 u16 u32 u64` | unsigned integers | |
| `i8 i16 i32 i64` | signed two's complement | |
| `index` | unsigned, natural word size of the target | array indices, counts |
| `f32 f64` | IEEE 754 binary32 / binary64 | gated by the target FPU policy (3.8) |
| `fixed<I, F>` | signed fixed point, `I` integer bits, `F` fraction bits | `q15 = fixed<1, 15>`, `q31 = fixed<1, 31>` |
| `ufixed<I, F>` | unsigned fixed point | |
| `bit<N>` | unsigned bit vector of width `N` (1 to 4096) | `bit = bit<1>`; registers and `hdl` |
| `sbit<N>` | signed bit vector | `hdl` |
| `duration` | signed time span, nanosecond resolution, 64 bit | literal `5 ms` |
| `instant` | monotonic point in time | `now()` |
| `str` | static read-only text | literals only |
| `str<N>` | text with fixed capacity `N` bytes | formatting, UI |

## 3.2 Compound types

| Form | Example | Meaning |
|---|---|---|
| Array | `[u8; 16]` | fixed length `N`, value semantics |
| Slice view | `[u8]` | parameter-only view of an array (05) |
| Tuple | `(f32, f32)` | |
| Range | `range<u16>`, `range<f32 degC>` | value of `a..b` or `a..=b`; `.start`, `.end`, `.contains(x)` |
| Option | `u16?` (sugar for `Option<u16>`) | value or `none` |
| Result | `Result<u16, IoError>` | value or error |
| Struct | `struct Point:` | named fields |
| Enum | `enum Mode:` | variants, optionally with payloads |
| Class | `class Pid:` | state + methods; lowers to IEC function blocks, C structs, HDL modules |
| Interface | `interface Sensor:` | method signatures; static dispatch only |
| Channel | `channel<Msg, 8>` | bounded queue (05) |
| Pool | `pool<Frame, 4>` | fixed-capacity allocation (05) |

```neuron
struct Point:
  x: f32 mm
  y: f32 mm

enum Command:
  Stop
  Move(Point, f32 mm/s)
  Home

class Debounce:
  let window: duration = 20 ms
  var stable: bool = false
  var since: instant = instant.zero

  pub fn update(mut self, raw: bool, t: instant) -> bool:
    if raw != self.stable and t - self.since >= self.window:
      self.stable = raw
      self.since = t
    elif raw == self.stable:
      self.since = t
    return self.stable

interface Sensor:
  fn read(mut self) -> Result<f32, IoError>
```

Generics use `<>` in type positions and are monomorphized:

```neuron
fn clamp<T: Ord>(x: T, lo: T, hi: T) -> T:
  requires lo <= hi
  ensures lo <= result <= hi
  return if x < lo then lo else if x > hi then hi else x

struct RingBuffer<T, const N: index>:
  data: [T; N]
  head: index = 0
```

## 3.3 Physical units

A **quantity type** is a numeric type followed by a unit expression:
`f32 degC`, `u16 mV`, `fixed<16,16> mm/s`, `i32 count`.
A bare unit (`degC`) means `f32 <unit>`.

Units form a group over **dimensions**. Neuron uses nine base dimensions:

| Dimension | Base unit |
|---|---|
| time | `s` |
| length | `m` |
| mass | `kg` |
| electric current | `A` |
| thermodynamic temperature | `K` |
| amount of substance | `mol` |
| luminous intensity | `cd` |
| plane angle | `rad` |
| count (items, pulses, steps) | `count` |

Ratio (`%`, `ppm`, `1`) is dimensionless.

Built-in units (with SI prefixes `n`, `u`/`µ`, `m`, `c`, `k`, `M`, `G` where
meaningful): `s min h`, `Hz rpm`, `m mm um in`, `g kg`, `N`, `Pa bar psi`, `J W`,
`V A Ohm F H C`, `K degC degF`, `rad deg rev`, `mol`, `cd lx`, `count`, `B` (byte),
`Bd` (baud = `1/s`), `%`, `ppm`. Packages MAY define units:
`unit step = count` or `unit lpm = L/min`.

### Rules

1. `+`, `-`, comparisons and assignment require **equal dimensions**. The compiler
   converts between compatible units exactly (`1 s + 250 ms` is `1.25 s`).
   Mixing dimensions is `NE0201`.
2. `*` and `/` combine dimensions (`mm / s` is velocity, `V / Ohm` is current).
3. Functions such as `sin`, `exp` and `log` require dimensionless arguments
   (angles are accepted by trigonometric functions).
4. **Affine units** (`degC`, `degF`): `degC - degC` yields `K` (a difference);
   `degC + K` yields `degC`; `degC + degC` is an error (`NE0203`).
5. Removing a unit is explicit: `(t / 1 ms) as u32` or `t.in(ms)`.
6. Integer quantities store values in the unit written in the type (`u16 mV` stores
   millivolts); conversions that are not exact in integer arithmetic are errors
   unless rounding is explicit (`.round()`).

> **Property UNIT-1.** For every well-typed expression, evaluating it after
> converting all inputs to base units yields the same value (within the declared
> numeric precision) as evaluating it in the declared units and converting the
> result.

## 3.4 Numeric semantics

- Arithmetic on `+ - * /` **traps on overflow** by default: in device dialects a
  trap raises `fault Overflow` (safe state, chapter 8); in `host` it aborts the
  program. Provable overflow at compile time is an error (`NE0207`).
- Wrapping (`+%`) and saturating (`+|`) operators are explicit.
- `@overflow(wrap | saturate | trap)` changes the default for a module or function.
  In `hdl` the default is `wrap` (hardware semantics).
- Integer division by zero behaves like overflow. Integer division truncates toward
  zero; `%` has the sign of the dividend.
- Shifts by a count ≥ the bit width are an error (static) or a trap (dynamic).
- `f32`/`f64` follow IEEE 754 round-to-nearest-even. `NaN` comparisons are false;
  `x.is_nan()` tests it. In device dialects, producing `NaN` from a sensor path
  SHOULD be handled; the lint `NL0201` warns on unchecked float division.
- Neuron has **no undefined behavior** in safe code. Every operation either has a
  defined result or a defined trap.

## 3.5 Conversions

| Conversion | How |
|---|---|
| Lossless (widening, int to exact float, unit change) | implicit for literals, `as` otherwise |
| Lossy integer narrowing | `.wrap()`, `.saturate()`, or `.try_into()` returning `T?` |
| Float to integer | `.round()`, `.floor()`, `.ceil()`, `.trunc()` then a lossless or checked step |
| Integer to fixed / fixed to float | `as`, lossless or reported (`NE0204`) |
| Bit reinterpretation | `bits.as_u16()`, `f32.to_bits()` (explicit, safe) |

`as` never loses information; a lossy `as` is `NE0204` with the right method suggested.

## 3.6 Type inference

- Local `let`/`var` types are inferred from the initializer.
- Integer literals take the type required by context; without context they default
  to `i32`, and real literals to `f32` (`NL0202` warns when the default is used in a
  device dialect on a target without FPU).
- Function signatures, module-level declarations and struct fields are always
  annotated (except `const`, which may be inferred).

## 3.7 Equality and ordering

`==` is structural for structs, enums, arrays and tuples whose elements support it.
`Ord` is defined for integers, fixed point, durations, instants and quantities.
Floats implement `PartialOrd` only; sorting floats requires an explicit total order.

## 3.8 Target numeric policy

The target description declares the FPU (`none`, `sp`, `dp`). On `none`:

- `f32`/`f64` are allowed but compiled with software emulation, and `NL0203`
  reports the estimated cost per operation;
- the compiler suggests `fixed<I,F>` with the required range and precision derived
  from the declared quantity ranges.
