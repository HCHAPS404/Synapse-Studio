# 16 · NIR (Neuron Intermediate Representation)

NIR is a Neuron package after parsing, name resolution and type and unit checking,
written as canonical JSON. It is the contract between the compiler and every other
tool: editors and graphical views, the digital twin, emitters, importers and the
interpreter. Chapter 11 §11.1 places NIR in the interchange standard; this chapter
defines it.

- **Schema.** The normative JSON Schema is
  [`spec/schemas/nir.schema.json`](../schemas/nir.schema.json) (JSON Schema 2020-12).
  It is generated from the implementation (`crates/neuron-nir`), and a test fails when
  the committed copy differs from the model.
- **Implementation.** `crates/neuron-nir` holds the data model, the canonical JSON
  writer and reader, the schema and the printer. It does not depend on the compiler,
  so editors and the twin can use it on their own, including from WebAssembly. The
  lowering from the analyzed program lives in `neuron-sema` (`lower.rs`).
- **Command line.** `neuron emit-nir <file|dir>... [-o out.nir.json]` and
  `neuron print-nir <file.nir.json>` (§16.8).

## 16.1 When NIR exists

NIR is produced only for a package with **no errors** (warnings are allowed). A
consumer can therefore rely on every reference being resolved and every check of
chapter 15 having passed. `Analysis::nir` is `None` otherwise, and `emit-nir` writes
nothing and exits with code 1.

## 16.2 Structure

NIR mirrors the structure of the source so that it can be printed back (§16.6). It
adds what the compiler knows: stable ids, resolved references, the type of every
expression and the exact value of constant quantities.

```text
Nir      { nir_version, edition, package?, modules: [Module] }
Module   { id, name, file, kind: module|cell, dialect, target?, items: [Item] }
Item     { id, span, public?, doc?, attrs?, def: ItemDef }
Expr     { span, ty: Type, e: ExprKind }
Stmt     { span, s: StmtKind }
Pattern  { span, p: PatternKind }
```

Every sum type is a JSON object with a **tag field** that names the variant, in
`snake_case`. The tag field names are short and fixed:

| Sum type | Tag field | Examples of tags |
|---|---|---|
| Item definition | `item` | `use`, `const`, `var`, `signal`, `fn`, `struct`, `class`, `task`, `interrupt`, `fsm`, `sequence`, `rung`, `safety`, `device`, `link` |
| Expression | `e` | `lit`, `quantity`, `ref`, `local`, `global`, `call`, `field`, `binary`, `compare`, `range`, `pipe` |
| Statement | `s` | `let`, `assign`, `if`, `match`, `for`, `while`, `return`, `wait`, `until`, `fault`, `emit` |
| Pattern | `p` | `wildcard`, `bind`, `lit`, `range`, `ctor`, `path`, `tuple`, `or` |
| Literal | `lit` | `int`, `real`, `str`, `bool`, `none`, `iec`, `bits`, `ip` |
| Resolved type | `t` | `bool`, `num`, `bits`, `array`, `adt`, `io`, `opaque`, `unknown` |
| Type as written | `t` | `named`, `array`, `slice`, `tuple` |
| Call target | `call` | `fn`, `method`, `ctor`, `io`, `builtin`, `foreign` |
| Task trigger | `trigger` | `every`, `on` |

The complete list of variants and fields is the schema.

## 16.3 Ids and references

Ids are stable: they depend only on the order of declarations, never on byte
positions, so editing a comment or a blank line does not change them.

| Id | Meaning |
|---|---|
| `m{i}` | Module `i`, in the order the files were given (a package directory is read in sorted path order) |
| `m{i}.i{k}` | Item `k` of module `i`, in source order |
| `m{i}.i{k}.f{n}` / `.m{n}` | Field `n` / method `n` of a class |
| `m{i}.i{k}.v{n}` | Variable `n` of a PLC section (`inputs`, `outputs`, `vars`) |
| `m{i}.i{k}.c{n}` | Clock `n` of a `clocks` block |
| `m{i}.l{n}` | Local binding `n` of module `i` (parameters, `let`, `var`, loop and pattern bindings), numbered in lowering order |

A name in an expression is lowered to what it refers to:

- `ref` — a definition of the package, with its item id.
- `local` — a local binding, with its local id.
- `module` — a module of the package (`use b` then `b.f()`).
- `state` — a state of a state machine, inside `verify`.
- `global` — a name the package does not define: the standard library, a dependency,
  a device description or a pin (`gpio`, `D13`, `ton`). These are resolved by later
  phases against their own manifests.

Every call records its `target` when the checker resolved it: a function (`fn`), a
method with the id of its class, a constructor (`ctor`), an operation on an `io`
binding, a prelude function block (`builtin`), or external vocabulary (`foreign`,
for example a call on another device in a cell).

## 16.4 Types and units

Every expression carries `ty`, the type computed by the checker (chapter 15 §15.2):

- `{"t": "num", "kind": "u16", "unit": {...}}` for numbers. `kind` is the numeric type
  (`u8` … `i64`, `f32`, `f64`) or `integer` / `real` for a literal whose type is not
  fixed by its context. `unit` is omitted for dimensionless numbers.
- A **unit** is `{"text", "dim", "scale", "offset"}`. `dim` lists the exponents of the
  base dimensions in the fixed order `s m kg A K mol cd rad count`. `scale` is the
  exact factor to the base unit and `offset` the exact offset (non-zero only for
  `degC` and `degF`, and omitted when zero). For example `ms` is
  `{"text": "ms", "dim": [1,0,0,0,0,0,0,0,0], "scale": "0.001"}`.
- `unknown` marks an expression whose meaning comes from outside the package: device
  and driver vocabulary (`gpio.output`, `ip(...)`, `zero_hop(...)`) and fields of
  devices in a cell. Device manifests (phase D1) and the standard library (A12) give
  these expressions their types; until then consumers treat them as opaque.

Types **as written** in declarations are kept separately (`ty` of a `const`, a
parameter or a field, as a type expression with its unit as written), next to the
resolved type (`vty` of a variable).

## 16.5 Exact numbers

Numbers are JSON **strings**, so no precision is lost between languages.

- A literal keeps its text as written and its exact value: `{"lit": "int", "text":
  "1_000", "v": "1000"}`. Hexadecimal, binary and octal integers are given in decimal
  in `v`.
- Exact values are decimals when the decimal expansion terminates (`0.001`) and the
  fraction `n/d` otherwise (`1/3`).
- A **quantity** keeps its number and its unit as written (`unit.text` and the unit
  factors), and adds `si`, the exact value in base SI units, when the quantity is a
  constant: `500 ms` has `"si": "0.5"`.
- Angles: the unit model defines `deg` and `rev` with π rounded to 15 significant
  digits, the only inexact constant of chapter 3. The `si` of an angle is exact with
  respect to that value (`90 deg` has `"si": "1.5707963267948965"`).
- An IPv4 address such as `ip(192.168.0.10)` is not a token (chapter 2 §2.8): the
  parser reads a real and two fields. Lowering recognizes four decimal octets
  (0 to 255) and records the literal `{"lit": "ip", "text": "192.168.0.10"}`.

## 16.6 Canonical JSON

- UTF-8, LF line endings, two-space indentation and a final newline.
- Object keys in schema order; arrays in source order. Optional fields and empty
  lists are omitted, and so are flags that are `false`.
- No floating-point numbers: every number is a string (§16.5). Spans are pairs of
  integers.
- `span` is a byte range `[start, end)` into the module's source file. Spans are the
  only part of NIR that depends on layout. Diagram positions are never in NIR; they
  live in `.layout.json`.

> **Property NIR-1 (canonical).** Analyzing the same package twice gives
> byte-identical NIR.

The formatter (task A11) extends NIR-1: a program and its formatted version give the
same NIR apart from spans.

## 16.7 Printer and round trip

The printer turns NIR back into Neuron source with one canonical layout: two-space
indentation, one blank line between top-level items, every block on its own lines,
and parentheses only where the precedence of chapter 4 §4.6 requires them. It keeps
names, numbers and units exactly as written. It does not keep ordinary comments or
the original spacing; documentation comments (`##`) are kept, because they are part
of NIR.

Two forms are printed specially so that they read back as the same NIR:

- A range whose bounds share a unit through the range rule (chapter 3 §3.3) is
  printed in its short form: `-40..40 mm`, never `-40 mm..40 mm`.
- An `ip` literal is printed as written: `ip(192.168.0.10)`.

> **Property P-RT (round trip).** For every package with no errors, printing its NIR
> and analyzing the printed source gives no errors and the same NIR, apart from
> spans. Printing that NIR again gives the same text (the printer is idempotent).

P-RT is tested on the `examples/` package, on every example on its own, on every
valid program of this specification, and on generated variations of the examples
(property-based).

## 16.8 Command line

```text
neuron emit-nir <file|dir>... [-o out.nir.json]
neuron print-nir <file.nir.json>
```

- `emit-nir` analyzes like `neuron check` and writes the canonical NIR to the file
  given with `-o`, or to standard output. Paths in `file` are relative to the package
  directory and use `/`, so the output does not depend on the machine.
- `print-nir` reads NIR (any `1.x`) and prints every module as source; with more than
  one module, each starts with a `# ---- file ----` comment line.

## 16.9 Versioning

- `nir_version` is `major.minor`, currently `1.0`. A minor version only adds optional
  fields or new variants that older readers can skip; a major version may change
  anything.
- Readers SHALL accept every `1.x` document and SHALL ignore fields they do not know.
  `neuron_nir::from_json` rejects other major versions.
- `edition` is the language edition (`2026`).
- The golden file `conformance/golden/nir/examples.nir.json` records the NIR of the
  examples. A change to it, or to the schema, is reviewed in the pull request that
  causes it.
