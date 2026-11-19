//! What an emitter returns: files, wiring report, fidelity report and source maps
//! (spec 11 §11.5 to §11.7), and the line writer that records the maps.

use crate::fidelity::Fidelity;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Source location of a mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Src {
    /// `.neuron` file.
    pub file: String,
    /// Byte range.
    pub span: [usize; 2],
}

/// One generated range and the source it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    /// `[line, column, end_line, end_column]`: lines from 1, columns from 0.
    pub gen: [usize; 4],
    /// Origin.
    pub src: Src,
    /// Graphical element (`rung:run_motor`, `step:Fill`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
}

/// `<file>.map.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMap {
    /// Format version (1).
    pub version: u32,
    /// Generated file name.
    pub generated: String,
    /// Mappings in generated order.
    pub mappings: Vec<Mapping>,
}

/// The output of an emitter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Generated files by relative path.
    pub files: BTreeMap<String, String>,
    /// What to connect and configure, in Markdown (`wiring.md`).
    pub wiring: String,
    /// `fidelity.json`.
    pub fidelity: Fidelity,
    /// One source map per generated file that has mappings.
    pub maps: BTreeMap<String, SourceMap>,
}

impl Package {
    /// An empty package with its fidelity report.
    pub fn new(fidelity: Fidelity) -> Package {
        Package {
            files: BTreeMap::new(),
            wiring: String::new(),
            fidelity,
            maps: BTreeMap::new(),
        }
    }

    /// Adds a file written with a [`Writer`], with its source map.
    pub fn add(&mut self, name: &str, w: Writer) {
        let (text, mappings) = w.finish();
        if !mappings.is_empty() {
            self.maps.insert(
                name.to_string(),
                SourceMap {
                    version: 1,
                    generated: name.to_string(),
                    mappings,
                },
            );
        }
        self.files.insert(name.to_string(), text);
    }

    /// Adds a file without a map.
    pub fn add_text(&mut self, name: &str, text: String) {
        self.files.insert(name.to_string(), text);
    }

    /// Every file of the package as it is written to disk, sorted by name:
    /// the generated files, `fidelity.json`, `wiring.md` and the source maps.
    pub fn all_files(&self) -> BTreeMap<String, String> {
        let mut out = self.files.clone();
        out.insert("fidelity.json".to_string(), self.fidelity.to_json());
        if !self.wiring.is_empty() {
            out.insert("wiring.md".to_string(), self.wiring.clone());
        }
        for (name, map) in &self.maps {
            out.insert(format!("{name}.map.json"), crate::to_json(map));
        }
        out
    }

    /// Writes [`Package::all_files`] under `dir`.
    ///
    /// # Errors
    /// Fails when a directory or file cannot be written.
    pub fn write(&self, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        for (name, text) in self.all_files() {
            let path = dir.join(&name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, text)?;
            written.push(path);
        }
        Ok(written)
    }
}

/// Writes indented lines and records where each mapped construct went.
#[derive(Debug, Default)]
pub struct Writer {
    text: String,
    indent: usize,
    unit: &'static str,
    line: usize,
    mappings: Vec<Mapping>,
}

/// Position at the start of a mapped range.
#[derive(Debug, Clone, Copy)]
pub struct Mark(usize);

impl Writer {
    /// A writer that indents with `unit` (for example four spaces).
    pub fn new(unit: &'static str) -> Writer {
        Writer {
            unit,
            line: 1,
            ..Writer::default()
        }
    }

    /// One more level of indentation.
    pub fn indent(&mut self) {
        self.indent += 1;
    }

    /// One level less.
    pub fn dedent(&mut self) {
        self.indent = self.indent.saturating_sub(1);
    }

    /// Writes one line at the current indentation (an empty line has no spaces).
    pub fn line(&mut self, s: &str) {
        if !s.is_empty() {
            for _ in 0..self.indent {
                self.text.push_str(self.unit);
            }
            self.text.push_str(s);
        }
        self.text.push('\n');
        self.line += 1;
    }

    /// Writes several lines (split at `\n`) at the current indentation.
    pub fn lines(&mut self, s: &str) {
        for l in s.lines() {
            self.line(l);
        }
    }

    /// Writes text exactly as given, without indentation.
    pub fn raw(&mut self, s: &str) {
        self.line += s.matches('\n').count();
        self.text.push_str(s);
    }

    /// Starts a mapped range at the next line.
    pub fn mark(&self) -> Mark {
        Mark(self.line)
    }

    /// Ends the range started at `m` and maps it to `src`.
    pub fn map(&mut self, m: Mark, file: &str, span: [usize; 2], view: Option<String>) {
        if self.line <= m.0 {
            return;
        }
        let end_line = self.line - 1;
        let end_col = self
            .text
            .trim_end_matches('\n')
            .rsplit('\n')
            .next()
            .map_or(0, str::len);
        self.mappings.push(Mapping {
            gen: [m.0, 0, end_line, end_col],
            src: Src {
                file: file.to_string(),
                span,
            },
            view,
        });
    }

    /// Appends what another writer wrote, moving its mappings to the lines they land on.
    pub fn append(&mut self, other: Writer) {
        let shift = self.line - 1;
        let (text, maps) = other.finish();
        self.line += text.matches('\n').count();
        self.text.push_str(&text);
        for mut m in maps {
            m.gen[0] += shift;
            m.gen[2] += shift;
            self.mappings.push(m);
        }
    }

    /// The text and the mappings.
    pub fn finish(self) -> (String, Vec<Mapping>) {
        (self.text, self.mappings)
    }
}
