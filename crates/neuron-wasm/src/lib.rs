//! Bindings of the Neuron compiler for the browser (WebAssembly) and for Python (PyO3).
//!
//! The [`api`] module is plain Rust: every function takes strings and returns a JSON
//! string, so it is tested natively with `cargo test`. When the crate is built for
//! `wasm32` the same functions are exported with wasm-bindgen (module `wasm`); the
//! `neuron-py` crate exports them to Python. Usage from JavaScript:
//! `packages/wasm-glue/README.md`.
//!
//! ```
//! let out = neuron_wasm::api::version();
//! assert!(out.contains("\"nir_schema\""));
//! ```

pub mod api;

#[cfg(target_arch = "wasm32")]
mod wasm;
