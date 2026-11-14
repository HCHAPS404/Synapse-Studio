//! Package manifest `neuron.toml` (spec 04 §4.7).
//!
//! Only the subset needed for name resolution is read: `[package]` and the names
//! in `[dependencies]`. The full manifest is handled by the package manager.

/// Contents of a `neuron.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Language edition.
    pub edition: String,
    /// Names of the dependencies.
    pub dependencies: Vec<String>,
}

impl Manifest {
    /// Parses the text of a `neuron.toml`.
    ///
    /// ```
    /// use neuron_sema::Manifest;
    /// let m = Manifest::parse("[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2026\"\n").unwrap();
    /// assert_eq!(m.name, "demo");
    /// ```
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let mut m = Manifest::default();
        let mut section = String::new();
        for (n, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim().to_string();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let name = rest
                    .strip_suffix(']')
                    .ok_or_else(|| format!("line {}: unclosed section header", n + 1))?;
                section = name.trim().to_string();
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {}: expected `key = value`", n + 1))?;
            let key = key.trim().trim_matches('"').to_string();
            let value = value.trim();
            match section.as_str() {
                "package" => {
                    let s = unquote(value)
                        .ok_or_else(|| format!("line {}: `{key}` must be a string", n + 1))?;
                    match key.as_str() {
                        "name" => m.name = s,
                        "version" => m.version = s,
                        "edition" => m.edition = s,
                        _ => {}
                    }
                }
                "dependencies" => m.dependencies.push(key),
                _ => {}
            }
        }
        if m.name.is_empty() {
            return Err("missing `name` in [package]".to_string());
        }
        if m.edition.is_empty() {
            return Err("missing `edition` in [package]".to_string());
        }
        Ok(m)
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(v: &str) -> Option<String> {
    let v = v.trim();
    v.strip_prefix('"')?.strip_suffix('"').map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_and_dependencies() {
        let t = "[package]\nname = \"bottling\" # comment\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\ndisplays = { git = \"https://example.org/x\", tag = \"v0.3.0\" }\n\n[targets.ctl]\nboard = \"nucleo_f446re\"\n";
        let m = Manifest::parse(t).unwrap_or_default();
        assert_eq!(m.name, "bottling");
        assert_eq!(m.edition, "2026");
        assert_eq!(m.dependencies, vec!["displays".to_string()]);
    }

    #[test]
    fn rejects_missing_name() {
        assert!(Manifest::parse("[package]\nedition = \"2026\"\n").is_err());
        assert!(Manifest::parse("[package\nname=\"x\"").is_err());
    }
}
