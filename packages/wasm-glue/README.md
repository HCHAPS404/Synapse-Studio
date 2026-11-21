# wasm-glue: the Neuron compiler in the browser

The compiler (analyzer, formatter, interpreter, emitters, importers) compiles to
WebAssembly from [`crates/neuron-wasm`](../../crates/neuron-wasm). Every function takes
strings and returns **one JSON string**, so the glue is a few lines and nothing is shared
by reference. The same functions exist for Python in `crates/neuron-py`.

## Build

```sh
rustup target add wasm32-unknown-unknown

# Option 1: wasm-pack (writes pkg/ with .wasm, .js and .d.ts)
cargo install wasm-pack
wasm-pack build crates/neuron-wasm --target web --release --out-dir ../../packages/wasm-glue/pkg

# Option 2: wasm-bindgen-cli (version must match the wasm-bindgen in Cargo.lock)
cargo install wasm-bindgen-cli
cargo build -p neuron-wasm --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/neuron_wasm.wasm \
  --target web --out-dir packages/wasm-glue/pkg
```

Use `--target bundler` for Next.js/webpack, or `--target web` with a manual `init()`.

## Use

```ts
import init, * as neuron from "./pkg/neuron_wasm.js";

await init();
const files = JSON.stringify({ "blink.neuron": source });
const report = JSON.parse(neuron.check(files));
if (!report.ok) throw new Error(report.error.message);
for (const d of report.diagnostics) {
  console.log(`${d.code} ${d.file}:${d.start.line}:${d.start.col} ${d.message}`);
}
```

## Conventions

- **Files** (`filesJson`): an object `{"<path>": "<text>"}` or an array
  `[{"path": "...", "text": "..."}]`. An entry called `neuron.toml` is the package manifest.
- **Success**: `{"ok": true, ...}`. **Failure**:
  `{"ok": false, "error": {"kind", "message"}, "diagnostics": [D]}`. `kind` is
  `bad_request`, `unknown_language`, `syntax`, `analysis`, `run`, `emit`, `import` or
  `internal`. No function throws on bad input.
- **Diagnostic D**: `{code, severity, message, help, file, span: [start, end],
  start: {line, col}, end: {line, col}, labels: [...], rendered}`. `span` is in UTF-8
  **bytes**; `line` and `col` are 1-based and count code points. `rendered` is the text
  `neuron check` prints. For Monaco (UTF-16 columns), convert when a line has characters
  above U+FFFF.
- **Offsets** passed to `hover`/`definition` are UTF-8 byte offsets.
- `endNs` of `run` is a JavaScript number of nanoseconds (exact up to 2^53).

## Functions

| Function | Reply (besides `ok`) |
|---|---|
| `check(filesJson)` | `has_errors`, `files`, `diagnostics` |
| `format(text)` | `text`, `changed`, `fell_back`; on a syntax error: `error.kind = "syntax"` and `diagnostics` |
| `hover(filesJson, file, offset)` | `hover`: `null` or `{markdown, start, end, range}` |
| `definition(filesJson, file, offset)` | `definition`: `null` or `{file, line, start, end, header, doc}` |
| `run(filesJson, endNs, scenario?)` | `end_ns`, `now_ns`, `fault`, `trace` (the text `neuron run` prints), `events`, `diagnostics` |
| `test(filesJson)` | `total`, `passed`, `failed`, `tests` |
| `emit(lang, filesJson, profile?)` | `lang`, `files` (path to text), `fidelity`, `diagnostics` |
| `importSource(lang, name, text)` | `source`, `complete`, `fidelity`, `diagnostics` |
| `version()` | `compiler`, `language_edition`, `nir_schema`, `protocol`, `emitters`, `importers` |

Emitters: `st`, `scl`, `plcopen`, `spel`, `python`, `kernel`, `c`. Importers: `st`, `scl`,
`plcopen`, `spel`, `c`, `verilog`. `hover` and `definition` analyze the package on each
call; debounce them.

## Examples

`check` with an error (`bad.neuron` is `module t\nfn f() -> u8:\n  return nope\n`; the
code and message are whatever `neuron check` reports, shown here as `NE####`):

```json
{"ok": true, "has_errors": true, "files": ["bad.neuron"],
 "diagnostics": [{"code": "NE####", "severity": "error", "message": "...", "help": null,
   "file": "bad.neuron", "span": [31, 35], "start": {"line": 3, "col": 10},
   "end": {"line": 3, "col": 14}, "labels": [], "rendered": "error[NE####]: ..."}]}
```

`format`: `format("module m for mcu\nio led:   gpio.output @ D13\n")`

```json
{"ok": true, "changed": true, "fell_back": false, "text": "module m for mcu\nio led: gpio.output @ D13\n"}
```

`hover(files, "t.neuron", 70)`:

```json
{"ok": true, "hover": {"markdown": "```neuron\nfn inc(x: u8) -> u8:\n```\n\nAdds one.\n\ntype: `u8`",
  "start": 68, "end": 71, "range": {"start": {"line": 6, "col": 10}, "end": {"line": 6, "col": 13}}}}
```

`definition(files, "t.neuron", 70)`:

```json
{"ok": true, "definition": {"file": "t.neuron", "line": 3, "start": {"line": 3, "col": 1},
  "end": {"line": 3, "col": 21}, "header": "fn inc(x: u8) -> u8:", "doc": "Adds one."}}
```

`run(files, 1500000000, "100 ms: button = true")`:

```json
{"ok": true, "end_ns": 1500000000, "now_ns": 1500000000, "fault": null,
 "trace": "      0.000 ms  OUT    led = false\n",
 "events": [{"t_ns": 0, "kind": "OUT", "name": "led", "value": "false"}], "diagnostics": []}
```

`test(files)`:

```json
{"ok": true, "total": 1, "passed": 0, "failed": 1,
 "tests": [{"file": "t.neuron", "module": 0, "name": "fails", "passed": false,
   "message": "assertion failed: one is not two",
   "at": {"file": "t.neuron", "span": [34, 40], "start": {"line": 3, "col": 3}, "end": {"line": 3, "col": 9}}}]}
```

`emit("st", files)`:

```json
{"ok": true, "lang": "st",
 "files": {"p06_pump_with_levels.st": "PROGRAM ...", "fidelity.json": "{...}", "wiring.md": "..."},
 "fidelity": {"summary": {"exact": 12, "equivalent": 1, "approximate": 0, "modeled": 0, "unsupported": 0}},
 "diagnostics": []}
```

`importSource("st", "latch", st)`:

```json
{"ok": true, "lang": "st", "name": "latch", "source": "module latch for plc ...",
 "complete": true, "fidelity": {"summary": {"exact": 3}}, "diagnostics": []}
```

`version()`:

```json
{"ok": true, "compiler": "0.0.1", "language_edition": "2026", "nir_schema": "1.0",
 "protocol": 1, "emitters": ["st", "scl", "plcopen", "spel", "python", "kernel", "c"],
 "importers": ["st", "scl", "plcopen", "spel", "c", "verilog"]}
```

The values in the examples are shortened; the exact shapes are asserted by
`crates/neuron-wasm/tests/api.rs`.

## Not included

There is no JavaScript build system here (no `package.json`): the TypeScript package that
wraps `pkg/` belongs to the web app work (G3).
