//! Program model: files, modules, symbols and imports (task A4, spec 15 §15.2).

use crate::manifest::Manifest;
use crate::prelude;
use crate::units::UnitTable;
use neuron_syntax::ast::{Dialect, File, GeometryKind, Ident, Item, ItemKind, PlcSection};
use neuron_syntax::{parse, Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

/// Key of a source location: `(file, start, end)`.
pub type SpanKey = (usize, usize, usize);

/// Builds a [`SpanKey`].
pub fn key(file: usize, span: Span) -> SpanKey {
    (file, span.start, span.end)
}

/// A diagnostic attached to a file of the analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiag {
    /// Index of the file in the analysis.
    pub file: usize,
    /// The diagnostic.
    pub diag: Diagnostic,
}

/// A source file given to the analysis.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Path used in diagnostics.
    pub path: String,
    /// Source text.
    pub text: String,
}

/// A parsed file.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    /// Path used in diagnostics.
    pub path: String,
    /// Source text.
    pub text: String,
    /// Syntax tree (always present, possibly with error nodes).
    pub ast: File,
    /// `true` when the parser reported errors.
    pub syntax_errors: bool,
}

/// What a module-level name denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymKind {
    /// `const`.
    Const,
    /// `var` or `let` at module level.
    Var,
    /// `signal`.
    Signal,
    /// `unit`.
    Unit,
    /// `event`.
    Event,
    /// `struct`.
    Struct,
    /// `enum`.
    Enum,
    /// `class`.
    Class,
    /// `interface`.
    Interface,
    /// `type` alias.
    Alias,
    /// `fn`.
    Fn,
    /// `task`.
    Task,
    /// `io` binding.
    Io,
    /// Clock (`clock` or an entry of `clocks:`).
    Clock,
    /// `fsm`.
    Fsm,
    /// `sequence`.
    Sequence,
    /// `rung`.
    Rung,
    /// `circuit`.
    Circuit,
    /// `frame`, `tool` or `point`.
    Geometry(GeometryKind),
    /// `display`.
    Display,
    /// `screen`.
    Screen,
    /// `theme`.
    Theme,
    /// `device`.
    Device,
    /// `tag`.
    Tag,
    /// `zone`.
    Zone,
    /// Variable of a plc section.
    PlcVar(PlcSection),
}

impl SymKind {
    /// `true` for names that live in the type namespace.
    pub fn is_type(self) -> bool {
        matches!(
            self,
            SymKind::Struct
                | SymKind::Enum
                | SymKind::Class
                | SymKind::Interface
                | SymKind::Alias
                | SymKind::Unit
        )
    }

    /// `true` for module-level mutable state that tasks may share.
    pub fn is_shared_state(self) -> bool {
        matches!(self, SymKind::Var | SymKind::Io | SymKind::PlcVar(_))
    }
}

/// A module-level definition.
#[derive(Debug, Clone)]
pub struct Symbol {
    /// Kind.
    pub kind: SymKind,
    /// `pub`.
    pub public: bool,
    /// Span of the name.
    pub span: Span,
    /// Index of the defining item in the file.
    pub item: usize,
    /// Index inside the item (plc variables, clock entries).
    pub sub: usize,
}

/// What an imported name refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    /// A whole package module.
    Module(usize),
    /// An item of a package module.
    Item {
        /// Module index.
        module: usize,
        /// Item name.
        name: String,
    },
    /// A name from the standard library or a dependency (typed as external vocabulary).
    Foreign(String),
}

/// An import with the span of its `use`.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// Target.
    pub import: Import,
    /// Span of the `use` item.
    pub span: Span,
}

/// Facts about one module (one file).
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Declared name (`a.b.c` or the cell name).
    pub name: String,
    /// Dialect.
    pub dialect: Dialect,
    /// Module-level definitions.
    pub symbols: BTreeMap<String, Symbol>,
    /// Imported names.
    pub imports: BTreeMap<String, ImportEntry>,
}

/// Result of resolving a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Res {
    /// A local binding (index into `Resolved::locals`).
    Local(usize),
    /// A module-level item.
    Item {
        /// Module index.
        module: usize,
        /// Item name.
        name: String,
    },
    /// A package module.
    Module(usize),
    /// An external name (standard library or dependency), by dotted path.
    Foreign(String),
    /// A prelude name.
    Prelude(String),
    /// A state of an `fsm` (inside `verify`).
    State {
        /// State machine.
        fsm: String,
        /// State.
        state: String,
    },
}

/// The whole program under analysis.
#[derive(Debug, Clone)]
pub struct Program {
    /// Parsed files.
    pub files: Vec<ParsedFile>,
    /// One module per file.
    pub modules: Vec<ModuleInfo>,
    /// Module index by declared name.
    pub by_name: BTreeMap<String, usize>,
    /// Units defined by the program.
    pub units: UnitTable,
    /// Package manifest.
    pub manifest: Option<Manifest>,
}

fn item_symbols(item: &Item) -> Vec<(Ident, SymKind, usize)> {
    let one = |n: &Ident, k: SymKind| vec![(n.clone(), k, 0)];
    match &item.kind {
        ItemKind::Const { name, .. } => one(name, SymKind::Const),
        ItemKind::Var(v) => one(&v.name, SymKind::Var),
        ItemKind::Signal { name, .. } => one(name, SymKind::Signal),
        ItemKind::Unit { name, .. } => one(name, SymKind::Unit),
        ItemKind::Event { name, .. } => one(name, SymKind::Event),
        ItemKind::Struct(d) => one(&d.name, SymKind::Struct),
        ItemKind::Enum(d) => one(&d.name, SymKind::Enum),
        ItemKind::Class(c) => one(&c.name, SymKind::Class),
        ItemKind::Interface { name, .. } => one(name, SymKind::Interface),
        ItemKind::Alias { name, .. } => one(name, SymKind::Alias),
        ItemKind::Fn(f) => one(&f.sig.name, SymKind::Fn),
        ItemKind::Task(t) => one(&t.name, SymKind::Task),
        ItemKind::Io { name, .. } => one(name, SymKind::Io),
        ItemKind::Clocks(defs) => defs
            .iter()
            .enumerate()
            .map(|(i, c)| (c.name.clone(), SymKind::Clock, i))
            .collect(),
        ItemKind::Fsm(f) => one(&f.name, SymKind::Fsm),
        ItemKind::Sequence(s) => one(&s.name, SymKind::Sequence),
        ItemKind::PlcSection { section, vars } => vars
            .iter()
            .enumerate()
            .map(|(i, v)| (v.name.clone(), SymKind::PlcVar(*section), i))
            .collect(),
        ItemKind::Rung { name, .. } => one(name, SymKind::Rung),
        ItemKind::Clock { name, .. } => one(name, SymKind::Clock),
        ItemKind::Circuit(c) => one(&c.name, SymKind::Circuit),
        ItemKind::Geometry { kind, name, .. } => one(name, SymKind::Geometry(*kind)),
        ItemKind::Display { name, .. } => one(name, SymKind::Display),
        ItemKind::Screen { name, .. } => one(name, SymKind::Screen),
        ItemKind::Theme { name, .. } => one(name, SymKind::Theme),
        ItemKind::Device { name, .. } => one(name, SymKind::Device),
        ItemKind::Tag { name, .. } => one(name, SymKind::Tag),
        ItemKind::Zone { name, .. } => one(name, SymKind::Zone),
        _ => Vec::new(),
    }
}

impl Program {
    /// Parses every file, registers modules and symbols, and resolves imports.
    ///
    /// Reports `NE0102` (duplicates), `NE0101`/`NE0105` (imports), `NE0104`
    /// (import cycles) and `NE0108` (invalid manifest is reported by the caller).
    pub fn build(
        sources: Vec<SourceFile>,
        manifest: Option<Manifest>,
        diags: &mut Vec<FileDiag>,
    ) -> Program {
        let mut files = Vec::new();
        for (i, s) in sources.into_iter().enumerate() {
            let parsed = parse(&s.text);
            let syntax_errors = parsed.diagnostics.iter().any(Diagnostic::is_error);
            for d in parsed.diagnostics {
                diags.push(FileDiag { file: i, diag: d });
            }
            files.push(ParsedFile {
                path: s.path,
                text: s.text,
                ast: parsed.file,
                syntax_errors,
            });
        }
        let mut prog = Program {
            files,
            modules: Vec::new(),
            by_name: BTreeMap::new(),
            units: UnitTable::new(),
            manifest,
        };
        prog.collect_modules(diags);
        prog.define_units();
        prog.resolve_imports(diags);
        prog.check_cycles(diags);
        prog
    }

    fn collect_modules(&mut self, diags: &mut Vec<FileDiag>) {
        for (i, f) in self.files.iter().enumerate() {
            let header = &f.ast.header;
            let name = header.name.dotted();
            let mut info = ModuleInfo {
                name: name.clone(),
                dialect: header.dialect,
                symbols: BTreeMap::new(),
                imports: BTreeMap::new(),
            };
            if let Some(&first) = self.by_name.get(&name) {
                diags.push(FileDiag {
                    file: i,
                    diag: Diagnostic::error(
                        "NE0102",
                        format!(
                            "module `{name}` is already defined in `{}`",
                            self.files[first].path
                        ),
                        header.name.span,
                    ),
                });
            } else {
                self.by_name.insert(name, i);
            }
            for (idx, item) in f.ast.items.iter().enumerate() {
                for (id, kind, sub) in item_symbols(item) {
                    if let Some(prev) = info.symbols.get(&id.name) {
                        diags.push(FileDiag {
                            file: i,
                            diag: Diagnostic::error(
                                "NE0102",
                                format!("`{}` is already defined in this module", id.name),
                                id.span,
                            )
                            .with_label(prev.span, "first definition here"),
                        });
                        continue;
                    }
                    info.symbols.insert(
                        id.name.clone(),
                        Symbol {
                            kind,
                            public: item.public,
                            span: id.span,
                            item: idx,
                            sub,
                        },
                    );
                }
            }
            self.modules.push(info);
        }
    }

    fn define_units(&mut self) {
        let mut pending: Vec<(String, neuron_syntax::ast::UnitExpr)> = Vec::new();
        for f in &self.files {
            for item in &f.ast.items {
                if let ItemKind::Unit { name, def } = &item.kind {
                    pending.push((name.name.clone(), def.clone()));
                }
            }
        }
        // Definitions may refer to each other in any order: iterate to a fixed point.
        for _ in 0..=pending.len() {
            let mut progress = false;
            let mut rest = Vec::new();
            for (name, def) in pending {
                match self.units.resolve(&def) {
                    Ok(u) => {
                        self.units.define(&name, u);
                        progress = true;
                    }
                    Err(_) => rest.push((name, def)),
                }
            }
            pending = rest;
            if pending.is_empty() || !progress {
                break;
            }
        }
    }

    /// Finds a package module by a dotted path (`self.` refers to the package root).
    pub fn find_module(&self, segs: &[String]) -> Option<usize> {
        if let Some(&i) = self.by_name.get(&segs.join(".")) {
            return Some(i);
        }
        if segs.first().map(String::as_str) == Some("self") {
            let rest = segs[1..].join(".");
            if let Some(&i) = self.by_name.get(&rest) {
                return Some(i);
            }
            if let Some(m) = &self.manifest {
                if let Some(&i) = self.by_name.get(&format!("{}.{rest}", m.name)) {
                    return Some(i);
                }
            }
        }
        None
    }

    fn foreign_status(&self, segs: &[String]) -> Result<bool, String> {
        match segs.first().map(String::as_str) {
            Some("std") => match segs.get(1) {
                Some(layer) if prelude::STD_LAYERS.contains(&layer.as_str()) => Ok(true),
                Some(layer) => Err(format!("unknown standard library layer `std.{layer}`")),
                None => Err("`std` is not a module; import `std.<layer>`".to_string()),
            },
            Some(first) => Ok(self
                .manifest
                .as_ref()
                .is_some_and(|m| m.dependencies.iter().any(|d| d == first))),
            None => Ok(false),
        }
    }

    fn resolve_imports(&mut self, diags: &mut Vec<FileDiag>) {
        let mut results: Vec<(usize, String, ImportEntry)> = Vec::new();
        for (mi, f) in self.files.iter().enumerate() {
            for item in &f.ast.items {
                let ItemKind::Use { path, names, alias } = &item.kind else {
                    continue;
                };
                let segs: Vec<String> = path.segments.iter().map(|s| s.name.clone()).collect();
                let mut report = |code: &'static str, msg: String| {
                    diags.push(FileDiag {
                        file: mi,
                        diag: Diagnostic::error(code, msg, item.span),
                    });
                };
                if !names.is_empty() {
                    if let Some(m) = self.find_module(&segs) {
                        for n in names {
                            match self.modules.get(m).and_then(|mm| mm.symbols.get(&n.name)) {
                                None => report(
                                    "NE0101",
                                    format!(
                                        "module `{}` has no item `{}`",
                                        self.modules[m].name, n.name
                                    ),
                                ),
                                Some(sym) => {
                                    if !sym.public && m != mi {
                                        report(
                                            "NE0105",
                                            format!(
                                                "`{}` is private to module `{}`",
                                                n.name, self.modules[m].name
                                            ),
                                        );
                                    }
                                    let bound = if names.len() == 1 {
                                        alias.as_ref().map_or(n.name.clone(), |a| a.name.clone())
                                    } else {
                                        n.name.clone()
                                    };
                                    results.push((
                                        mi,
                                        bound,
                                        ImportEntry {
                                            import: Import::Item {
                                                module: m,
                                                name: n.name.clone(),
                                            },
                                            span: item.span,
                                        },
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                    match self.foreign_status(&segs) {
                        Ok(true) => {
                            for n in names {
                                let bound = if names.len() == 1 {
                                    alias.as_ref().map_or(n.name.clone(), |a| a.name.clone())
                                } else {
                                    n.name.clone()
                                };
                                results.push((
                                    mi,
                                    bound,
                                    ImportEntry {
                                        import: Import::Foreign(format!(
                                            "{}.{}",
                                            segs.join("."),
                                            n.name
                                        )),
                                        span: item.span,
                                    },
                                ));
                            }
                        }
                        Ok(false) => {
                            report("NE0101", format!("unknown module `{}`", segs.join(".")))
                        }
                        Err(msg) => report("NE0101", msg),
                    }
                    continue;
                }
                let last = segs.last().cloned().unwrap_or_default();
                let bound = alias.as_ref().map_or(last.clone(), |a| a.name.clone());
                if let Some(m) = self.find_module(&segs) {
                    results.push((
                        mi,
                        bound,
                        ImportEntry {
                            import: Import::Module(m),
                            span: item.span,
                        },
                    ));
                    continue;
                }
                if segs.len() > 1 {
                    if let Some(m) = self.find_module(&segs[..segs.len() - 1]) {
                        match self.modules.get(m).and_then(|mm| mm.symbols.get(&last)) {
                            None => report(
                                "NE0101",
                                format!("module `{}` has no item `{last}`", self.modules[m].name),
                            ),
                            Some(sym) => {
                                if !sym.public && m != mi {
                                    report(
                                        "NE0105",
                                        format!(
                                            "`{last}` is private to module `{}`",
                                            self.modules[m].name
                                        ),
                                    );
                                }
                                results.push((
                                    mi,
                                    bound,
                                    ImportEntry {
                                        import: Import::Item {
                                            module: m,
                                            name: last,
                                        },
                                        span: item.span,
                                    },
                                ));
                            }
                        }
                        continue;
                    }
                }
                match self.foreign_status(&segs) {
                    Ok(true) => results.push((
                        mi,
                        bound,
                        ImportEntry {
                            import: Import::Foreign(segs.join(".")),
                            span: item.span,
                        },
                    )),
                    Ok(false) => {
                        report("NE0101", format!("unresolved import `{}`", segs.join(".")))
                    }
                    Err(msg) => report("NE0101", msg),
                }
            }
        }
        for (mi, name, entry) in results {
            let module = &mut self.modules[mi];
            if let Some(sym) = module.symbols.get(&name) {
                diags.push(FileDiag {
                    file: mi,
                    diag: Diagnostic::error(
                        "NE0102",
                        format!("imported name `{name}` clashes with a definition in this module"),
                        entry.span,
                    )
                    .with_label(sym.span, "defined here"),
                });
                continue;
            }
            if let Some(prev) = module.imports.get(&name) {
                if prev.import != entry.import {
                    diags.push(FileDiag {
                        file: mi,
                        diag: Diagnostic::error(
                            "NE0102",
                            format!("`{name}` is imported twice with different meanings"),
                            entry.span,
                        )
                        .with_label(prev.span, "first import here"),
                    });
                }
                continue;
            }
            module.imports.insert(name, entry);
        }
    }

    fn check_cycles(&self, diags: &mut Vec<FileDiag>) {
        let n = self.modules.len();
        let mut edges: Vec<Vec<(usize, Span)>> = vec![Vec::new(); n];
        for (i, m) in self.modules.iter().enumerate() {
            let mut seen = BTreeSet::new();
            for e in m.imports.values() {
                let target = match &e.import {
                    Import::Module(t) => *t,
                    Import::Item { module, .. } => *module,
                    Import::Foreign(_) => continue,
                };
                if seen.insert((target, e.span.start)) {
                    edges[i].push((target, e.span));
                }
            }
        }
        // 0 = unvisited, 1 = on the stack, 2 = done.
        let mut color = vec![0u8; n];
        let mut stack: Vec<usize> = Vec::new();
        for start in 0..n {
            if color[start] == 0 {
                self.dfs(start, &edges, &mut color, &mut stack, diags);
            }
        }
    }

    fn dfs(
        &self,
        node: usize,
        edges: &[Vec<(usize, Span)>],
        color: &mut [u8],
        stack: &mut Vec<usize>,
        diags: &mut Vec<FileDiag>,
    ) {
        color[node] = 1;
        stack.push(node);
        for &(next, span) in &edges[node] {
            if color[next] == 1 {
                let from = stack.iter().position(|&x| x == next).unwrap_or(0);
                let mut cycle: Vec<String> = stack[from..]
                    .iter()
                    .map(|&i| self.modules[i].name.clone())
                    .collect();
                cycle.push(self.modules[next].name.clone());
                diags.push(FileDiag {
                    file: node,
                    diag: Diagnostic::error(
                        "NE0104",
                        format!("circular import: {}", cycle.join(" -> ")),
                        span,
                    ),
                });
            } else if color[next] == 0 {
                self.dfs(next, edges, color, stack, diags);
            }
        }
        stack.pop();
        color[node] = 2;
    }

    /// Looks up a module-level name: own symbols, then imports, then the prelude.
    pub fn lookup(&self, module: usize, name: &str) -> Option<Res> {
        let m = self.modules.get(module)?;
        if m.symbols.contains_key(name) {
            return Some(Res::Item {
                module,
                name: name.to_string(),
            });
        }
        if let Some(e) = m.imports.get(name) {
            return Some(match &e.import {
                Import::Module(i) => Res::Module(*i),
                Import::Item { module, name } => Res::Item {
                    module: *module,
                    name: name.clone(),
                },
                Import::Foreign(p) => Res::Foreign(p.clone()),
            });
        }
        if prelude::is_prelude(name) {
            return Some(Res::Prelude(name.to_string()));
        }
        None
    }

    /// The symbol an `Item` resolution refers to.
    pub fn symbol(&self, module: usize, name: &str) -> Option<&Symbol> {
        self.modules.get(module)?.symbols.get(name)
    }
}
