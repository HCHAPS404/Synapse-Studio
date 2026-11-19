//! The fidelity report (spec 11 §11.4) and the project policy over it.

use crate::Diag;
use serde::{Deserialize, Serialize};
use std::fmt;

/// How faithfully a construct was translated, from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Same observable behavior by construction.
    Exact,
    /// Same observable behavior up to a documented difference.
    Equivalent,
    /// Close; the difference is quantified in the entry.
    Approximate,
    /// Represented as a black box with a declared contract.
    Modeled,
    /// Cannot be represented; reported and not translated.
    Unsupported,
}

impl Class {
    /// Reads `exact`, `equivalent`, `approximate`, `modeled` or `unsupported`.
    pub fn parse(s: &str) -> Option<Class> {
        Some(match s {
            "exact" => Class::Exact,
            "equivalent" => Class::Equivalent,
            "approximate" => Class::Approximate,
            "modeled" => Class::Modeled,
            "unsupported" => Class::Unsupported,
            _ => return None,
        })
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Class::Exact => "exact",
            Class::Equivalent => "equivalent",
            Class::Approximate => "approximate",
            Class::Modeled => "modeled",
            Class::Unsupported => "unsupported",
        })
    }
}

/// Number of constructs per class.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Exact.
    pub exact: u32,
    /// Equivalent.
    pub equivalent: u32,
    /// Approximate.
    pub approximate: u32,
    /// Modeled.
    pub modeled: u32,
    /// Unsupported.
    pub unsupported: u32,
}

/// One construct that was not translated exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Source file of the construct.
    pub file: String,
    /// Byte range in that file.
    pub span: [usize; 2],
    /// Class.
    pub class: Class,
    /// What the construct is (`timer`, `sequence`, `move arc`).
    pub construct: String,
    /// The difference, in words and with its bound.
    pub note: String,
}

/// `fidelity.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fidelity {
    /// Tool that wrote the report (`emit-st`, `import-st`).
    pub tool: String,
    /// Version of the tool.
    pub version: String,
    /// Target (`plc.generic`, `robot.spel`, `neuron.plc`).
    pub target: String,
    /// Counts per class.
    pub summary: Summary,
    /// Every construct that is not exact, in source order.
    pub entries: Vec<Entry>,
}

impl Fidelity {
    /// An empty report.
    pub fn new(tool: &str, target: &str) -> Fidelity {
        Fidelity {
            tool: tool.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            target: target.to_string(),
            summary: Summary::default(),
            entries: Vec::new(),
        }
    }

    /// Counts one construct translated exactly.
    pub fn exact(&mut self) {
        self.summary.exact += 1;
    }

    /// Records one construct; anything but `exact` gets an entry.
    pub fn record(
        &mut self,
        file: &str,
        span: [usize; 2],
        class: Class,
        construct: &str,
        note: &str,
    ) {
        match class {
            Class::Exact => self.summary.exact += 1,
            Class::Equivalent => self.summary.equivalent += 1,
            Class::Approximate => self.summary.approximate += 1,
            Class::Modeled => self.summary.modeled += 1,
            Class::Unsupported => self.summary.unsupported += 1,
        }
        if class != Class::Exact {
            self.entries.push(Entry {
                file: file.to_string(),
                span,
                class,
                construct: construct.to_string(),
                note: note.to_string(),
            });
        }
    }

    /// Adds the counts and entries of another report.
    pub fn merge(&mut self, other: Fidelity) {
        self.summary.exact += other.summary.exact;
        self.summary.equivalent += other.summary.equivalent;
        self.summary.approximate += other.summary.approximate;
        self.summary.modeled += other.summary.modeled;
        self.summary.unsupported += other.summary.unsupported;
        self.entries.extend(other.entries);
    }

    /// The worst class in the report.
    pub fn worst(&self) -> Class {
        self.entries
            .iter()
            .map(|e| e.class)
            .max()
            .unwrap_or(Class::Exact)
    }

    /// Canonical JSON text.
    pub fn to_json(&self) -> String {
        crate::to_json(self)
    }
}

/// What a project accepts: nothing worse than `worst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// The worst class the project accepts.
    pub worst: Class,
}

impl Policy {
    /// One `NE1003` per entry worse than the policy.
    pub fn check(&self, f: &Fidelity) -> Vec<Diag> {
        f.entries
            .iter()
            .filter(|e| e.class > self.worst)
            .map(|e| Diag {
                code: "NE1003",
                message: format!(
                    "`{}` is translated as {}, but the project accepts nothing worse than {}",
                    e.construct, e.class, self.worst
                ),
                help: Some(e.note.clone()),
                file: e.file.clone(),
                span: e.span,
            })
            .collect()
    }
}
