//! `test "name":` blocks and the `std.test` functions (`drive`, `advance`, ...).

use crate::calls::Vals;
use crate::value::Value;
use crate::{fault, Interp, Options, Stop, R};
use neuron_nir::{ItemDef, ModuleKind, Nir};
use std::collections::BTreeSet;

/// Result of one `test` block.
#[derive(Debug, Clone)]
pub struct TestReport {
    /// Index of the module that contains the test.
    pub module: usize,
    /// Test name.
    pub name: String,
    /// `true` when every assertion held and the body ran to the end.
    pub passed: bool,
    /// Why it failed (empty when it passed).
    pub message: String,
    /// Source location of the failed assertion: module index and byte range.
    pub at: Option<(usize, [usize; 2])>,
}

impl Interp {
    /// Runs `f` without losing the locals and context of the code that called it.
    fn nested(&mut self, f: impl FnOnce(&mut Interp)) {
        let locals = std::mem::take(&mut self.locals);
        let names = std::mem::take(&mut self.names);
        let ctx = std::mem::take(&mut self.ctx);
        let module = self.cur_module;
        f(self);
        self.locals = locals;
        self.names = names;
        self.ctx = ctx;
        self.cur_module = module;
    }

    /// `drive(target, value)`: sets an input now and lets the program react.
    pub(crate) fn test_drive(&mut self, vals: &Vals) -> R<Value> {
        let (Some((_, target)), Some((_, value))) = (vals.first(), vals.get(1)) else {
            return Err(fault("ContractViolation"));
        };
        let value = value.clone();
        match target {
            Value::Io(k) => {
                let k = k.clone();
                self.nested(|s| {
                    s.apply_input(&k, value);
                    s.drain_ready();
                });
            }
            Value::Str(n) | Value::Opaque(n) => {
                let n = n.clone();
                self.nested(|s| {
                    s.apply_named_input(&n, &value);
                    s.drain_ready();
                });
            }
            _ => return Err(fault("ContractViolation")),
        }
        Ok(Value::Unit)
    }

    /// `advance(duration)`: lets simulated time pass.
    pub(crate) fn test_advance(&mut self, vals: &Vals) -> R<Value> {
        let secs = vals
            .first()
            .and_then(|(_, v)| v.as_f64())
            .ok_or_else(|| fault("ContractViolation"))?;
        let end = self.now.saturating_add((secs * 1.0e9).round() as i64);
        self.nested(|s| s.run_until(end));
        Ok(Value::Unit)
    }
}

/// For each module, the other modules whose items it refers to.
///
/// Items refer to each other by id (`m3.i2`), so a module refers to module `m3` when its
/// canonical JSON contains a string that starts with `m3.i`.
fn module_refs(nir: &Nir) -> Vec<BTreeSet<usize>> {
    let marks: Vec<String> = nir
        .modules
        .iter()
        .map(|md| format!("\"{}.i", md.id))
        .collect();
    nir.modules
        .iter()
        .enumerate()
        .map(|(k, md)| {
            let text = neuron_nir::to_json(&Nir {
                nir_version: nir.nir_version.clone(),
                edition: nir.edition.clone(),
                package: None,
                modules: vec![md.clone()],
            });
            marks
                .iter()
                .enumerate()
                .filter(|(j, mark)| *j != k && text.contains(mark.as_str()))
                .map(|(j, _)| j)
                .collect()
        })
        .collect()
}

/// Modules a test of module `m` runs, in package order: `m` and every module it refers
/// to, directly or through other modules. A test in a cell runs the whole package
/// (chapter 17.8).
fn test_scope(nir: &Nir, refs: &[BTreeSet<usize>], m: usize) -> Vec<usize> {
    if nir.modules[m].kind == ModuleKind::Cell {
        return (0..nir.modules.len()).collect();
    }
    let mut seen = BTreeSet::from([m]);
    let mut todo = vec![m];
    while let Some(k) = todo.pop() {
        for &j in &refs[k] {
            if seen.insert(j) {
                todo.push(j);
            }
        }
    }
    seen.into_iter().collect()
}

/// Runs every `test` block of a package, each on a fresh copy of its module and the
/// modules that module refers to (chapter 17.8), so a fault in an unrelated module
/// cannot stop it.
pub fn run_tests(nir: &Nir, opts: &Options) -> Vec<TestReport> {
    let mut out = Vec::new();
    let refs = module_refs(nir);
    for (m, module) in nir.modules.iter().enumerate() {
        if !module
            .items
            .iter()
            .any(|i| matches!(i.def, ItemDef::Test { .. }))
        {
            continue;
        }
        let scope = test_scope(nir, &refs, m);
        let pos = scope.iter().position(|&j| j == m).unwrap_or(0);
        let program = Nir {
            nir_version: nir.nir_version.clone(),
            edition: nir.edition.clone(),
            package: nir.package.clone(),
            modules: scope.iter().map(|&j| nir.modules[j].clone()).collect(),
        };
        for item in &module.items {
            let ItemDef::Test { name, body } = &item.def else {
                continue;
            };
            let name = name.trim_matches('"').to_string();
            let mut report = TestReport {
                module: m,
                name,
                passed: true,
                message: String::new(),
                at: None,
            };
            match Interp::new(program.clone(), opts.clone()) {
                Err(e) => {
                    report.passed = false;
                    report.message = e.message;
                }
                Ok(mut it) => {
                    it.in_test = true;
                    it.cur_module = pos;
                    match it.exec_block(body) {
                        Ok(()) | Err(Stop::Return(_) | Stop::Break | Stop::Continue) => {}
                        Err(Stop::Fault(f)) => {
                            report.passed = false;
                            match it.fail.take() {
                                Some((fm, span, text)) => {
                                    report.message = if text.is_empty() {
                                        "assertion failed".to_string()
                                    } else {
                                        format!("assertion failed: {text}")
                                    };
                                    // `fm` counts modules of the scope; report the package index.
                                    report.at = Some((scope.get(fm).copied().unwrap_or(m), span));
                                }
                                None => report.message = format!("the program raised fault {f}"),
                            }
                        }
                    }
                }
            }
            out.push(report);
        }
    }
    out
}
