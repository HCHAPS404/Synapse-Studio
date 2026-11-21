# neuron-py

PyO3 module `neuron_py`: the compiler API of [`neuron-wasm`](../neuron-wasm) for the Python
services (Core API, AI gateway). Every function takes strings and returns a JSON string;
the shapes are in `crates/neuron-wasm/src/api.rs` and `packages/wasm-glue/README.md`.

| Function | Returns |
|---|---|
| `check(files_json)` | diagnostics of a package |
| `format(text)` | canonical text, or the syntax errors |
| `hover(files_json, file, offset)` / `definition(...)` | declaration and type under the cursor |
| `run(files_json, end_ns, scenario=None)` | trace text and structured events |
| `test(files_json)` | test report |
| `emit(lang, files_json, profile=None)` | generated files, fidelity, diagnostics |
| `import_source(lang, name, text)` | Neuron source, fidelity, diagnostics |
| `version()` | compiler, NIR schema and protocol versions |

## Build

The crate is **not** a member of the cargo workspace (`exclude` in the root `Cargo.toml`):
PyO3 needs a Python interpreter when it builds, so `cargo test --workspace` does not
touch it. Build it with maturin, from this directory:

```powershell
uv tool install maturin          # once
maturin develop --release        # installs neuron_py into the active virtualenv
maturin build --release          # writes a wheel to target/wheels
python -c "import neuron_py; print(neuron_py.version())"
```

`cargo check` also works (`cargo check` in this directory, with a Python 3.12 on the
`PATH`); the `extension-module` feature is only needed to produce the loadable module,
and maturin enables it.
