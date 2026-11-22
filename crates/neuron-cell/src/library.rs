//! The set of device and medium manifests, and the resolution of a cell against it.
use crate::manifest::{invalid, Kind, Manifest, Medium};
use neuron_nir::{ExprKind, ItemDef, Nir};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Failure to load or use a manifest. Every variant is reported as `NE0607`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// A file could not be read or parsed.
    #[error("cannot read {file}: {message}")]
    Parse {
        /// File name.
        file: String,
        /// Parser message.
        message: String,
    },
    /// A file is well formed but wrong.
    #[error("{file}: {message}")]
    Invalid {
        /// File name.
        file: String,
        /// What is wrong.
        message: String,
    },
    /// A name that no manifest defines.
    #[error("unknown {kind} `{name}`{}", hint(.known))]
    Unknown {
        /// `device` or `medium`.
        kind: &'static str,
        /// The missing name.
        name: String,
        /// Names that exist.
        known: Vec<String>,
    },
}

fn hint(known: &[String]) -> String {
    if known.is_empty() {
        String::new()
    } else {
        format!(" (known: {})", known.join(", "))
    }
}

impl ManifestError {
    /// The diagnostic code (`NE0607`).
    pub fn code(&self) -> &'static str {
        "NE0607"
    }
}

/// Manifests compiled into the tool: `(path under devices/, text)`.
const BUILTIN: &[(&str, &str)] = &[
    (
        "media/modbus_tcp.toml",
        include_str!("../../../devices/media/modbus_tcp.toml"),
    ),
    (
        "media/s7.toml",
        include_str!("../../../devices/media/s7.toml"),
    ),
    (
        "media/zero_hop.toml",
        include_str!("../../../devices/media/zero_hop.toml"),
    ),
    (
        "plcs/generic_s7_1200.toml",
        include_str!("../../../devices/plcs/generic_s7_1200.toml"),
    ),
    (
        "plcs/generic_iec_plc.toml",
        include_str!("../../../devices/plcs/generic_iec_plc.toml"),
    ),
    (
        "controllers/nucleo_f446re.toml",
        include_str!("../../../devices/controllers/nucleo_f446re.toml"),
    ),
    (
        "fpgas/generic_artix7_board.toml",
        include_str!("../../../devices/fpgas/generic_artix7_board.toml"),
    ),
    (
        "robots/generic_6axis.toml",
        include_str!("../../../devices/robots/generic_6axis.toml"),
    ),
    (
        "robots/generic_scara.toml",
        include_str!("../../../devices/robots/generic_scara.toml"),
    ),
];

/// A device resolved from a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDevice {
    /// Instance name in the cell (`plc`).
    pub instance: String,
    /// Manifest name (`generic_s7_1200`).
    pub manifest: String,
}

/// A medium used by a link, with the options written after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMedium {
    /// Medium name (`zero_hop`).
    pub medium: String,
    /// Options (`gpio`, `opto_24v`).
    pub options: Vec<String>,
}

/// Everything a cell names, found in the library.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellManifests {
    /// Devices in source order.
    pub devices: Vec<ResolvedDevice>,
    /// One entry per link, in source order.
    pub media: Vec<ResolvedMedium>,
}

/// Device and medium manifests by name.
#[derive(Debug, Clone, Default)]
pub struct Library {
    devices: BTreeMap<String, Manifest>,
    media: BTreeMap<String, Medium>,
}

impl Library {
    /// The manifests compiled into the tool.
    ///
    /// ```
    /// let lib = neuron_cell::Library::builtin().unwrap();
    /// assert!(lib.device("generic_scara").is_some());
    /// assert!(lib.medium("modbus_tcp").is_some());
    /// ```
    ///
    /// # Errors
    /// `NE0607` if a built-in file is invalid (a bug of the tool).
    pub fn builtin() -> Result<Library, ManifestError> {
        let mut lib = Library::default();
        for (path, text) in BUILTIN {
            lib.add_text(path, text)?;
        }
        lib.check()?;
        Ok(lib)
    }

    /// Loads a `devices/` directory (any class folder may be missing).
    ///
    /// # Errors
    /// `NE0607` for an unreadable or invalid file.
    pub fn from_dir(root: &Path) -> Result<Library, ManifestError> {
        let mut lib = Library::default();
        lib.overlay_dir(root)?;
        Ok(lib)
    }

    /// Adds the manifests of a `devices/` directory, replacing those with the same name.
    /// The library is unchanged when this fails.
    ///
    /// # Errors
    /// `NE0607` for an unreadable or invalid file.
    pub fn overlay_dir(&mut self, root: &Path) -> Result<(), ManifestError> {
        let mut next = self.clone();
        let mut folders: Vec<&str> = vec!["media"];
        let classes = [Kind::Plc, Kind::Mcu, Kind::Fpga, Kind::Robot, Kind::Host];
        folders.extend(classes.iter().map(|k| k.folder()));
        for folder in folders {
            let dir = root.join(folder);
            if !dir.is_dir() {
                continue;
            }
            let io = |e: std::io::Error| ManifestError::Parse {
                file: dir.display().to_string(),
                message: e.to_string(),
            };
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .map_err(io)?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .collect();
            files.sort();
            for f in files {
                let text = std::fs::read_to_string(&f).map_err(|e| ManifestError::Parse {
                    file: f.display().to_string(),
                    message: e.to_string(),
                })?;
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                next.add_text(&format!("{folder}/{name}"), &text)?;
            }
        }
        next.check()?;
        *self = next;
        Ok(())
    }

    /// Adds one manifest; `path` is `<folder>/<name>.toml` under `devices/`. Call
    /// [`Library::check`] after the last one.
    ///
    /// # Errors
    /// `NE0607` if the file is invalid, sits in the wrong folder or its name differs
    /// from the file stem.
    pub fn add_text(&mut self, path: &str, text: &str) -> Result<(), ManifestError> {
        let (folder, file) = path
            .split_once('/')
            .ok_or_else(|| invalid(path, "expected `<folder>/<name>.toml`"))?;
        let stem = file.strip_suffix(".toml").unwrap_or(file);
        if folder == "media" {
            let m = Medium::parse(path, text)?;
            if m.name != stem {
                return Err(invalid(
                    path,
                    format!("name `{}` differs from the file name", m.name),
                ));
            }
            self.media.insert(m.name.clone(), m);
            return Ok(());
        }
        let class = Kind::from_folder(folder)
            .ok_or_else(|| invalid(path, format!("unknown folder `{folder}`")))?;
        let m = Manifest::parse(path, text)?;
        if m.kind != class {
            return Err(invalid(
                path,
                format!("kind {:?} does not belong in `{folder}`", m.kind),
            ));
        }
        if m.name != stem {
            return Err(invalid(
                path,
                format!("name `{}` differs from the file name", m.name),
            ));
        }
        self.devices.insert(m.name.clone(), m);
        Ok(())
    }

    /// Checks the references between manifests: boards, FPGA boards and the media named
    /// in timing tables.
    ///
    /// # Errors
    /// `NE0607` for the first dangling reference.
    pub fn check(&self) -> Result<(), ManifestError> {
        let registry = neuron_target::Registry::builtin().map_err(|e| ManifestError::Parse {
            file: "targets".into(),
            message: e.to_string(),
        })?;
        for m in self.devices.values() {
            let file = format!("{}/{}.toml", m.kind.folder(), m.name);
            if let Some(t) = &m.target {
                registry
                    .resolve(t)
                    .map_err(|e| invalid(&file, format!("device.target: {e}")))?;
            }
            if let Some(b) = &m.board {
                if neuron_codegen_hdl::FpgaBoard::builtin(b).is_none() {
                    return Err(invalid(
                        &file,
                        format!("device.board: unknown FPGA board `{b}`"),
                    ));
                }
            }
            for medium in m.timing.keys() {
                let known = self
                    .media
                    .get(medium)
                    .ok_or_else(|| invalid(&file, format!("timing: unknown medium `{medium}`")))?;
                if !known.speaks.contains(&m.kind) {
                    return Err(invalid(
                        &file,
                        format!(
                            "timing: medium `{medium}` is not spoken by {:?} devices",
                            m.kind
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// A device manifest by name.
    pub fn device(&self, name: &str) -> Option<&Manifest> {
        self.devices.get(name)
    }

    /// A medium by name.
    pub fn medium(&self, name: &str) -> Option<&Medium> {
        self.media.get(name)
    }

    /// Device manifest names in order.
    pub fn device_names(&self) -> Vec<String> {
        self.devices.keys().cloned().collect()
    }

    /// Medium names in order.
    pub fn medium_names(&self) -> Vec<String> {
        self.media.keys().cloned().collect()
    }

    /// Finds the manifest of every device and medium that the cells of `nir` name.
    ///
    /// # Errors
    /// One `NE0607` per unknown device, unknown medium or unknown medium option.
    pub fn resolve_cell(&self, nir: &Nir) -> Result<CellManifests, Vec<ManifestError>> {
        let mut out = CellManifests::default();
        let mut errors = Vec::new();
        let mut seen = BTreeSet::new();
        for module in nir
            .modules
            .iter()
            .filter(|m| m.kind == neuron_nir::ModuleKind::Cell)
        {
            for item in &module.items {
                match &item.def {
                    ItemDef::Device { name, ty, .. } => {
                        if self.devices.contains_key(ty) {
                            out.devices.push(crate::library::ResolvedDevice {
                                instance: name.clone(),
                                manifest: ty.clone(),
                            });
                        } else if seen.insert(ty.clone()) {
                            errors.push(ManifestError::Unknown {
                                kind: "device",
                                name: ty.clone(),
                                known: self.device_names(),
                            });
                        }
                    }
                    ItemDef::Link { via, .. } => match self.medium_of(&module.file, via) {
                        Ok(m) => out.media.push(m),
                        Err(e) => errors.push(e),
                    },
                    _ => {}
                }
            }
        }
        if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        }
    }

    fn medium_of(
        &self,
        file: &str,
        via: &neuron_nir::Expr,
    ) -> Result<ResolvedMedium, ManifestError> {
        let (head, args): (&neuron_nir::Expr, &[neuron_nir::Arg]) = match &via.e {
            ExprKind::Call { callee, args, .. } => (callee, args),
            _ => (via, &[]),
        };
        let name_of = |e: &neuron_nir::Expr| match &e.e {
            ExprKind::Global { name }
            | ExprKind::Ref { name, .. }
            | ExprKind::Local { name, .. } => Some(name.clone()),
            _ => None,
        };
        let name = name_of(head).ok_or_else(|| invalid(file, "`via` must name a medium"))?;
        let medium = self
            .media
            .get(&name)
            .ok_or_else(|| ManifestError::Unknown {
                kind: "medium",
                name: name.clone(),
                known: self.medium_names(),
            })?;
        let mut options = Vec::new();
        for a in args {
            let o = name_of(&a.value).ok_or_else(|| {
                invalid(file, format!("medium `{name}`: arguments must be names"))
            })?;
            if !medium.options.contains(&o) {
                return Err(invalid(
                    file,
                    format!(
                        "medium `{name}` has no option `{o}` (options: {})",
                        medium.options.join(", ")
                    ),
                ));
            }
            options.push(o);
        }
        Ok(ResolvedMedium {
            medium: name,
            options,
        })
    }
}
