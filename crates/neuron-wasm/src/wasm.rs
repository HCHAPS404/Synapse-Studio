//! wasm-bindgen exports of [`crate::api`] (compiled for `wasm32` only).
//!
//! The code that `#[wasm_bindgen]` generates contains `unsafe`, so this module is the one
//! place of the crate where the `unsafe_code` lint is allowed. It has no `unsafe` of its
//! own. Every function takes and returns strings; see `packages/wasm-glue/README.md`.
#![allow(unsafe_code)]

use crate::api;
use wasm_bindgen::prelude::wasm_bindgen;

/// `check(filesJson)`: diagnostics of a package.
#[wasm_bindgen]
pub fn check(files_json: &str) -> String {
    api::check(files_json)
}

/// `format(text)`: the canonical formatting of one source text.
#[wasm_bindgen]
pub fn format(text: &str) -> String {
    api::format(text)
}

/// `hover(filesJson, file, offset)`: declaration and type under the cursor.
#[wasm_bindgen]
pub fn hover(files_json: &str, file: &str, offset: usize) -> String {
    api::hover(files_json, file, offset)
}

/// `definition(filesJson, file, offset)`: where the name under the cursor is declared.
#[wasm_bindgen]
pub fn definition(files_json: &str, file: &str, offset: usize) -> String {
    api::definition(files_json, file, offset)
}

/// `run(filesJson, endNs, scenario)`: runs the program in the reference interpreter.
/// `endNs` is a number of nanoseconds (a JavaScript number, exact up to 2^53).
#[wasm_bindgen]
pub fn run(files_json: &str, end_ns: f64, scenario: Option<String>) -> String {
    api::run(files_json, end_ns as i64, scenario.as_deref())
}

/// `test(filesJson)`: runs every `test` block.
#[wasm_bindgen]
pub fn test(files_json: &str) -> String {
    api::test(files_json)
}

/// `emit(lang, filesJson, profile)`: translates the program to another language.
#[wasm_bindgen]
pub fn emit(lang: &str, files_json: &str, profile: Option<String>) -> String {
    api::emit(lang, files_json, profile.as_deref())
}

/// `importSource(lang, name, text)`: reads text in another language as Neuron source.
#[wasm_bindgen(js_name = importSource)]
pub fn import_source(lang: &str, name: &str, text: &str) -> String {
    api::import(lang, name, text)
}

/// `version()`: versions of the compiler and its formats.
#[wasm_bindgen]
pub fn version() -> String {
    api::version()
}
