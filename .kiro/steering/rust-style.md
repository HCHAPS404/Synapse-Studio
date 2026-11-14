---
inclusion: fileMatch
fileMatchPattern: "**/*.rs"
---

# Rust style

- Edition 2021 or newer, `#![forbid(unsafe_code)]` by default. `unsafe` only in
  crates that need it (FFI, engine hot paths), documented with a `// SAFETY:`
  comment per block.
- Errors: `thiserror` for libraries, typed error enums carrying `NE####` codes for
  compiler diagnostics. No `unwrap()`/`expect()` outside tests and examples.
- Deterministic by default: no `HashMap` iteration order dependence in compiler
  output (use `IndexMap`/`BTreeMap`), seeded RNG only.
- `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` must pass.
- Property tests with `proptest` for parsers, formatters, passes and solvers.
- Public items documented with `///` including at least one example where useful.
