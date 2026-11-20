//! Robot manifests (`devices/robots/*.toml`): the limits that `NE0804` checks targets against.
//!
//! ```
//! let r = neuron_emit::robot::Robot::builtin("generic_6axis").unwrap();
//! assert_eq!(r.joint_limits_deg.len(), 6);
//! assert!(r.reach_mm > 0.0);
//! ```

use serde::Deserialize;

const GENERIC_6AXIS: &str = include_str!("../../../devices/robots/generic_6axis.toml");
const GENERIC_SCARA: &str = include_str!("../../../devices/robots/generic_scara.toml");

/// Names of the manifests compiled into the emitter.
pub fn builtin_names() -> &'static [&'static str] {
    &["generic_6axis", "generic_scara"]
}

/// The limits of one robot model.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Robot {
    /// Model name (`generic_6axis`).
    pub name: String,
    /// `articulated` or `scara`.
    pub kind: String,
    /// Maximum distance from the shoulder point to the flange, in mm.
    pub reach_mm: f64,
    /// Height of the shoulder point above the base plane, in mm.
    pub shoulder_height_mm: f64,
    /// Lowest allowed flange height in the base frame, in mm.
    pub z_min_mm: f64,
    /// Highest allowed flange height in the base frame, in mm.
    pub z_max_mm: f64,
    /// `[min, max]` of every joint, in degrees.
    pub joint_limits_deg: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
struct File {
    robot: Robot,
}

impl Robot {
    /// Reads a manifest from TOML text.
    ///
    /// # Errors
    /// The message of the TOML parser.
    pub fn parse(text: &str) -> Result<Robot, String> {
        toml::from_str::<File>(text)
            .map(|f| f.robot)
            .map_err(|e| e.to_string())
    }

    /// A manifest compiled into the emitter (`generic_6axis`, `generic_scara`).
    pub fn builtin(name: &str) -> Option<Robot> {
        let text = match name {
            "generic_6axis" => GENERIC_6AXIS,
            "generic_scara" => GENERIC_SCARA,
            _ => return None,
        };
        Robot::parse(text).ok()
    }

    /// A built-in manifest, or a `.toml` file when `name` ends with `.toml`.
    ///
    /// # Errors
    /// Unknown name, unreadable file or invalid manifest.
    pub fn load(name: &str) -> Result<Robot, String> {
        if name.ends_with(".toml") {
            let text = std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?;
            return Robot::parse(&text).map_err(|e| format!("{name}: {e}"));
        }
        let n = if name.is_empty() {
            "generic_6axis"
        } else {
            name
        };
        Robot::builtin(n).ok_or_else(|| {
            format!(
                "unknown robot manifest `{n}` (built in: {})",
                builtin_names().join(", ")
            )
        })
    }
}
