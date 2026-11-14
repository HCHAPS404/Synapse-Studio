# Syntax errors (NE0001 to NE0016)

Source of truth for the catalogue: `spec/language/10-diagnostics.md`. This page gives
the explanation and the fix for each lexical and syntax error. `neuron explain NE0006`
prints the matching section.

## NE0001 tab character used for indentation
Indentation uses 2 spaces per level. Replace the tab with spaces; `neuron fmt` does it.

## NE0002 dedent does not match any enclosing indentation level
A line returns to an indentation that no open block has. Align it with one of the
enclosing blocks.

```text
if ready:
    start()
  stop()      # NE0002: 2 spaces matches no enclosing level
```

## NE0003 unexpected token
The parser found a token that cannot continue the construct. The message names the
token and what was expected. A common cause is an operator or unit in the wrong place.

## NE0004 unterminated string or block comment
Close the string with `"`, or close the `#[ ... ]#` comment. Block comments nest, so
each `#[` needs its own `]#`.

## NE0005 expected an indented block after `:`
A header ending in `:` needs at least one indented line below it. In a cell file the
items must be indented under `cell Name:`.

## NE0006 invalid numeric literal
Typical causes and fixes:

| Written | Problem | Write |
|---|---|---|
| `5ms` | number glued to its unit | `5 ms` |
| `.5` | no digit before the point | `0.5` |
| `5.` | no digit after the point | `5.0` |
| `1__0`, `1_000_`, `0x_FF` | misplaced underscore | `1_0`, `1_000`, `0xFF` |

## NE0007 invalid escape sequence
Valid escapes are `\n \r \t \\ \" \0 \xHH \u{HHHH}`, plus `\{` and `\}` for literal
braces in interpolated strings. Use one of them.

## NE0008 invalid character or byte literal
A character literal (`'A'`) holds exactly one ASCII character. An empty literal or a
non-ASCII character is an error; use a string for text.

## NE0009 unexpected indentation at top level
A top-level item starts at column 0. Remove the leading spaces, or indent it under the
construct that should own it.

## NE0010 unknown dialect in module header
The dialect after `for` is one of `core`, `mcu`, `plc`, `hdl`, `robot`, `ui`, `host`.
A `cell` is a separate file kind, not a dialect.

## NE0011 unclosed bracket
A `(`, `[` or `{` is never closed. The error points at the opening bracket.

## NE0012 contract outside the start of a function body
`requires` and `ensures` must be the first lines of the function body. `invariant`
belongs in a class body. Move the line.

## NE0013 content after the `cell` block
A cell file contains one cell. Move any other item inside the block or into another
file.

## NE0014 number and unit are not separated by exactly one space
Write `5 ms`. Two spaces and a tab are errors.

## NE0015 non-ASCII character in a name
Names use ASCII letters, digits and `_`. Only the unit characters `µ`, `Ω` and `°`
are accepted outside strings and comments.

## NE0016 invalid `{...}` interpolation in a string
An interpolation must contain a valid expression and be closed. To write a literal
brace, escape it: `\{` and `\}`.
