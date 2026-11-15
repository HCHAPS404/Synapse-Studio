# 02 · Lexical structure

## 2.1 Source text

- Source files SHALL be UTF-8 with extension `.neuron`. A byte-order mark is
  ignored. Line endings `LF` or `CRLF` are both accepted and normalized to `LF`.
- Identifiers, keywords and operators are ASCII. Non-ASCII characters are allowed in
  strings, comments and unit names only (the unit characters `µ`, `Ω` and `°`, see
  2.7). Any other use of them in a name is `NE0015`.

## 2.2 Lines, indentation and blocks

- A **logical line** ends at a newline that is not inside `()`, `[]` or `{}`.
  Inside brackets, newlines and indentation are ignored (implicit continuation). A
  bracket that is never closed is `NE0011`, reported where it opens.
- A block is opened by `:` at the end of a logical line followed by a line with
  greater indentation. The lexer emits `INDENT` and `DEDENT` tokens.
- The indentation unit is **2 spaces**. Tabs in indentation are an error (`NE0001`).
  Inconsistent dedent (not matching an enclosing level) is an error (`NE0002`).
- Blank lines and comment-only lines do not affect indentation.
- A short block MAY stay on the header line: `if x > 3: led.on()`. Several simple
  statements MAY share a line separated by `;`.

```neuron
task t every 10 ms:
  if level > 80 %: pump.off()
  counter += 1; flag = true
```

## 2.3 Comments

| Form | Meaning |
|---|---|
| `# text` | Line comment |
| `## text` | Documentation comment, attached to the next item (rendered by `neuron doc`) |
| `#[ ... ]#` | Block comment, nestable |

## 2.4 Identifiers

`identifier = [A-Za-z_][A-Za-z0-9_]*`. Identifiers are case sensitive.

Style (lint `NL0001`, not an error): `UpperCamel` for types, states, steps, faults
and enum variants; `snake_case` for everything else; `SCREAMING_SNAKE` is allowed
for constants. Hardware location names (`PA5`, `USART2`, `W5`, `D13`) are
identifiers resolved in the target's namespace.

## 2.5 Keywords (edition 2026)

Reserved in every dialect:

```text
and       as        assert    atomic    break     cell      class     const
continue  elif      else      emit      enum      extern    fault     fn
for       if        in        interface io        is        let       loop
match     module    mut       not       on        or        pass      pub
requires  ensures   invariant return    safety    self      signal    struct
task      test      then      trusted   type      use       var       verify
when      while     true      false     none
```

`pass` is the empty statement. `max`, `impl` and `Self` are **not** reserved: `max` is
an ordinary name (and a contextual word after `while` and `loop`); `impl` and `Self`
are free for later editions.

Contextual keywords (reserved only where their dialect or construct allows them):

| Context | Words |
|---|---|
| tasks and rules | `every`, `offset`, `prio`, `deadline`, `core`, `reads`, `writes`, `stack`, `interrupt`, `after` |
| state machines and sequences | `fsm`, `state`, `initial`, `final`, `entry`, `exit`, `do`, `sequence`, `step`, `parallel`, `branch`, `run`, `until`, `timeout`, `wait` |
| safety | `safe_state`, `watchdog`, `rate`, `reset` |
| `verify` blocks | `never`, `always`, `implies`, `eventually_within` |
| loops | `max` (`while c max N`, `loop max N`) |
| contracts | `result`, `old` |
| packages | `unit` |
| `mcu` | `clocks`, `from`, `regs`, `uses`, `asm` |
| `plc` | `scan`, `inputs`, `outputs`, `memory`, `retain`, `rung`, `set` |
| `hdl` | `clock`, `circuit`, `reg`, `wire`, `comb`, `rising`, `falling`, `sync`, `via`, `instance`, `out`, `inout` |
| `robot` | `frame`, `tool`, `point`, `move`, `ptp`, `linear`, `arc`, `to`, `speed`, `accel`, `blend` |
| `ui` | `display`, `screen`, `bind`, `at`, `size`, `row`, `column`, `grid`, `theme` |
| `cell` | `device`, `link`, `via`, `within`, `zone`, `mutex`, `between`, `place`, `event`, `tag` |

A contextual keyword can be used as an identifier outside its context; inside its
context, escape it with backticks: `` `step` ``.

## 2.6 Literals

### Integers
```text
42        1_000_000     0x2A      0xFF_FF     0b1010_0000     0o755
```
An integer literal has no intrinsic type; its type comes from context (see 3.6).
A literal that does not fit the inferred type is an error (`NE0205`).
Underscores separate digits, one at a time: `1__0`, `1_000_`, `0x_FF` are `NE0006`.

### Reals
```text
3.14      0.5     1e-3     2.5e6     6.022e23
```
A digit is required on both sides of the decimal point. `.5` and `5.` are `NE0006`
with the fixes `0.5` and `5.0`. In `t.0.1` and `0..8` the dots are field access and
range operators, not decimal points.

### Booleans and none
`true`, `false`, `none` (the empty value of `T?`).

### Strings
```text
"Hello\n"     "Temp: {temp:.1} degC"     r"C:\raw\no\escapes"
```
- Escapes: `\n \r \t \\ \" \0 \xHH \u{HHHH}`, and `\{` and `\}` for literal braces.
- **Interpolation**: `{expr}` or `{expr:spec}` inside a normal string is formatted
  at compile time into a fixed-size buffer; it never allocates (spec: `.N` decimals,
  `x` hex, `b` binary, `>N` width). A `{` without its `}`, a nested `{` or a lone
  `}` is `NE0016`.
- String literals have type `str` (static, read-only, stored in flash on Harvard
  targets). Raw strings (`r"..."`) and byte strings (`b"..."`) have no interpolation.

### Characters and bytes
`'A'` is a `u8` (ASCII only); `b"\x01\x02"` is a `[u8; N]` byte string.

## 2.7 Quantities (numbers with units)

A **quantity literal** is a number followed, on the same line and separated by
**exactly one space**, by a **unit expression**:

```text
5 ms     500 ms     24 V     3.3 V     120 mm     0.5 mm/s     9.81 m/s^2
1 kHz    115200 Bd   60 degC  50 %     2 s      15 N*m        1.2 kOhm
```

- Grammar: `unit_expr = unit_atom (("*" | "/") unit_atom)*`,
  `unit_atom = unit_name ("^" "-"? digit+)?`.
- More than one space (or a tab) between the number and the unit is `NE0014`; the
  unit is still read, so the program is otherwise checked normally.
- A number immediately followed by letters is always a mistake (`5ms`): `NE0006`
  with the fix `5 ms`.
- A unit suffix may follow a number, a parenthesized tuple, a call (vector
  constructors such as `xyz(300, 0, 0) mm`, `rpy(0, 0, 0) deg`, `joints(...) deg`
  apply the unit to every component) and, through the range rule (03 §3.3), a range.
  For any other call the semantic phase reports `NE0202`: write the unit on the
  arguments instead, for example `box(x: 100..220 mm)`.
- **`%`** is a unit when no operand follows it (`50 %`, `0..100 %, antiwindup`) and
  the remainder operator when one does (`7 % 2`, `f(x) % n`).
- Unit aliases: `us` and `µs`; `Ohm` and `Ω`; `deg` and `°`; `degC` and `°C`.

## 2.8 Hardware literals

| Literal | Example | Dialect |
|---|---|---|
| IEC 61131-3 direct address | `%I0.0`, `%Q0.1`, `%IW64`, `%MD10`, `%DB1.DBX0.0` | `plc`, `cell` |
| Bit vector (sized) | `8'b1010_0000`, `16'hBEEF`, `4'd9` | `hdl` |
| IP address | `ip(192.168.0.10)` (a call, not a token; NIR records an `ip` literal, chapter 16 §16.5) | all |

## 2.9 Operators and punctuation

```text
+   -   *   /   %   **        arithmetic (trap on overflow by default)
+%  -%  *%                    wrapping
+|  -|  *|                    saturating
&   |   ^   ~   <<  >>        bitwise
==  !=  <   <=  >   >=        comparison (chainable: 0 <= x < 10)
and  or  not                  logical (short-circuit)
=   +=  -=  *=  /=  %=  &=  |=  ^=  <<=  >>=     assignment
|>                            pipeline: x |> f(a) is f(x, a)
++                            bit-vector concatenation (hdl)
??                            option default: x ?? fallback
->                            transition, mapping, link direction, return type
=>                            match arm
..  ..=                       range (exclusive / inclusive end)
?                             option type suffix and error propagation
@                             placement (location) and attributes
:   ,   ;   .   ( ) [ ] { }
```

`{ }` are used only in import lists (`use std.hal.{gpio, uart}`) and string
interpolation; they never delimit blocks.

A call or index needs its `(` or `[` **directly** after the callee: `f(x)` is a call,
`f (x)` is two expressions (this is what lets `task t on ev (prio 1):` work).

## 2.10 Properties

> **Property LEX-1 (round trip).** For every token sequence produced by the lexer,
> concatenating the token texts with the original trivia (spaces, comments,
> newlines) reproduces the source byte for byte.
>
> **Property LEX-2 (indentation balance).** For every source accepted by the lexer,
> the number of `INDENT` tokens equals the number of `DEDENT` tokens, and no prefix
> of the stream has more `DEDENT` than `INDENT`.

## 2.11 Changes from v0.1

`max`, `impl` and `Self` are no longer reserved; `result` and `old` are contextual;
`.5`, `5.`, `1__0` and `5  ms` are diagnosed; strings validate `{...}`;
non-ASCII names are rejected; `%` is disambiguated; `in(5)` and `out(3)` in `robot`
are now `din(5)` and `dout(3)`.
