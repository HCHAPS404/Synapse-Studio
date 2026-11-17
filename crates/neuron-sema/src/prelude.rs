//! Names that are always in scope (the prelude) and the standard-library layers.
//!
//! The standard library itself (`lib/`) is written in task A12. Until then the
//! prelude lists the well-known names so that programs can use them, and their
//! signatures are treated as external vocabulary (type `Unknown`).

/// Primitive and built-in generic type names.
pub const TYPE_NAMES: &[&str] = &[
    "bool",
    "u8",
    "u16",
    "u32",
    "u64",
    "i8",
    "i16",
    "i32",
    "i64",
    "index",
    "f32",
    "f64",
    "fixed",
    "ufixed",
    "bit",
    "sbit",
    "duration",
    "instant",
    "str",
    "range",
    "Option",
    "Result",
    "Handle",
    "channel",
    "pool",
    "list",
    "map",
    "text",
    "point",
    "frame",
    "tool",
    "IoError",
    "ConfigError",
    "Ord",
    "PartialOrd",
    "Eq",
    "Copy",
    "Num",
];

/// Bounds that the compiler knows how to check.
pub const BOUND_NAMES: &[&str] = &["Ord", "PartialOrd", "Eq", "Copy", "Num"];

/// Function blocks of `std.logic` and `std.control`.
pub const STD_BLOCKS: &[&str] = &[
    "ton",
    "tof",
    "tp",
    "ctu",
    "ctd",
    "ctud",
    "r_trig",
    "f_trig",
    "sr",
    "rs",
    "pid",
    "lowpass",
    "highpass",
    "moving_average",
    "hysteresis",
    "debounce",
    "ramp",
];

/// Free functions of the standard library.
pub const STD_FUNCTIONS: &[&str] = &[
    "abs",
    "min",
    "max",
    "clamp",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sqrt",
    "exp",
    "log",
    "log10",
    "floor",
    "ceil",
    "round",
    "pow",
    "hypot",
    "sign",
    "now",
    "scale",
    // `std.test`: legal in `test` blocks (spec 17 section 17.6).
    "drive",
    "advance",
    "elapsed",
    "current_fault",
    "state_of",
];

/// Constructors for geometry and results.
pub const CONSTRUCTORS: &[&str] = &["Ok", "Err", "xyz", "xy", "rpy", "joints", "frame", "tool"];

/// Vector constructors that accept a unit after the call.
pub const VECTOR_CONSTRUCTORS: &[&str] = &["xyz", "xy", "rpy", "joints"];

/// Peripheral kinds from `std.hal`.
pub const IO_KINDS: &[&str] = &[
    "gpio", "adc", "dac", "pwm", "timer", "capture", "encoder", "uart", "spi", "i2c", "can",
    "usb_cdc", "i2s", "rtc", "watchdog",
];

/// Layers of the standard library (spec 12 §12.1).
pub const STD_LAYERS: &[&str] = &[
    "core", "units", "time", "logic", "hal", "control", "motion", "proto", "dsp", "drivers",
    "safety", "diag", "test", "bsp", "ui", "display",
];

/// `true` when `name` is a prelude type name.
pub fn is_type_name(name: &str) -> bool {
    TYPE_NAMES.contains(&name)
}

/// `true` when `name` is a prelude value (function, block, constructor or peripheral kind).
pub fn is_value_name(name: &str) -> bool {
    STD_BLOCKS.contains(&name)
        || STD_FUNCTIONS.contains(&name)
        || CONSTRUCTORS.contains(&name)
        || IO_KINDS.contains(&name)
}

/// `true` when `name` is in the prelude at all.
pub fn is_prelude(name: &str) -> bool {
    is_type_name(name) || is_value_name(name)
}
