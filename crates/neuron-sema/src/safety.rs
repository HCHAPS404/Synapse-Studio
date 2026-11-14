//! Safe-state checks (task A8, spec 08 §8.1).
//!
//! * `NE0802`: every output-capable binding appears in `safe_state`.
//! * `NE0803`: `on fault` does not drive outputs.
//!
//! Model checking of state machines and sequences (`NE0901` to `NE0904`) is in `mc`.

use crate::check::Checked;
use crate::facts::{root_name, BodyKind, FactBase};
use crate::model::{FileDiag, Program, SymKind};
use crate::ty::Ty;
use crate::visit::each_stmt;
use neuron_syntax::ast::{Dialect, ExprKind, PlcSection, Stmt, StmtKind};
use neuron_syntax::{Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

fn is_output_kind(kind: &str) -> bool {
    matches!(kind, "gpio.output" | "gpio.open_drain" | "pwm" | "dac")
}

fn read_like(method: &str) -> bool {
    matches!(
        method,
        "read" | "ready" | "byte" | "rising" | "falling" | "changed"
    )
}

/// Names driven by the statements (assignments, coils and method calls).
fn driven(stmts: &[&Stmt], only_writes: bool) -> Vec<(String, Span)> {
    let mut out = Vec::new();
    for s in stmts {
        each_stmt(std::slice::from_ref(*s), &mut |x| match &x.kind {
            StmtKind::Assign { place, .. } | StmtKind::Coil { place, .. } => {
                if let Some(r) = root_name(place) {
                    out.push((r.name.clone(), x.span));
                }
            }
            StmtKind::Expr(e) => {
                if let ExprKind::Call(callee, _) = &e.kind {
                    if let ExprKind::Field(recv, m) = &callee.kind {
                        if !(only_writes && read_like(&m.name)) {
                            if let Some(r) = root_name(recv) {
                                out.push((r.name.clone(), x.span));
                            }
                        }
                    }
                }
            }
            _ => {}
        });
    }
    out
}

/// Runs the safe-state checks.
pub fn check_safety(
    prog: &Program,
    checked: &Checked,
    facts: &FactBase<'_>,
    diags: &mut Vec<FileDiag>,
) {
    for (mi, m) in prog.modules.iter().enumerate() {
        if prog.files[mi].syntax_errors
            || !matches!(m.dialect, Dialect::Mcu | Dialect::Plc | Dialect::Robot)
        {
            continue;
        }
        // Output-capable bindings of the module.
        let mut outputs: BTreeMap<String, Span> = BTreeMap::new();
        for (name, sym) in &m.symbols {
            let is_out = match sym.kind {
                SymKind::Io => matches!(
                    checked.item_ty.get(&(mi, name.clone())),
                    Some(Ty::Io(k)) if is_output_kind(k)
                ),
                SymKind::PlcVar(PlcSection::Outputs) => true,
                _ => false,
            };
            if is_out {
                outputs.insert(name.clone(), sym.span);
            }
        }
        if outputs.is_empty() {
            continue;
        }
        let mut covered: BTreeSet<String> = BTreeSet::new();
        for b in facts.bodies.iter().filter(|b| b.module == mi) {
            if b.kind == BodyKind::SafeState {
                for (n, _) in driven(&b.stmts, false) {
                    covered.insert(n);
                }
            }
        }
        for (name, span) in &outputs {
            if !covered.contains(name) {
                diags.push(FileDiag {
                    file: mi,
                    diag: Diagnostic::error(
                        "NE0802",
                        format!("output `{name}` has no safe state"),
                        *span,
                    )
                    .with_help(format!(
                        "add it to `safe_state:` in a `safety:` block, for example `{name}.write(false)`"
                    )),
                });
            }
        }
        for b in facts
            .bodies
            .iter()
            .filter(|b| b.module == mi && b.kind == BodyKind::OnFault)
        {
            for (n, span) in driven(&b.stmts, true) {
                if outputs.contains_key(&n) {
                    diags.push(FileDiag {
                        file: mi,
                        diag: Diagnostic::error(
                            "NE0803",
                            format!("`on fault` drives output `{n}`"),
                            span,
                        )
                        .with_help(
                            "the safe state is already applied; `on fault` may only log or notify",
                        ),
                    });
                }
            }
        }
    }
}
