//! Scenarios: scripted inputs for a run.
//!
//! One step per line. Blank lines and lines starting with `#` are ignored.
//!
//! ```text
//! at 100 ms: button = true        # drive an input (an io binding, a PLC input, a device field)
//! 2 s: sensor = 2048              # `at` is optional
//! 3 s: temp = 95 degC             # a number with a unit
//! 4 s: timer2.update!             # raise an event or a hardware event
//! 5 s: press start                # press a widget of a `ui` module
//! ```

use crate::value::{UnitInfo, Value};
use std::rc::Rc;

/// What a step does.
#[derive(Debug, Clone)]
pub enum Action {
    /// Sets an input to a value.
    Set(String, Value),
    /// Raises an event.
    Pulse(String),
    /// Presses a widget.
    Press(String),
}

/// A step of a scenario.
#[derive(Debug, Clone)]
pub struct Step {
    /// Instant of the step in nanoseconds.
    pub t_ns: i64,
    /// The action.
    pub action: Action,
}

/// A list of steps.
#[derive(Debug, Clone, Default)]
pub struct Scenario {
    /// Steps in file order.
    pub steps: Vec<Step>,
}

/// Parses `500 ms`, `2s`, `1.5 s`, `10 us`, `3 min` into nanoseconds.
pub fn parse_duration(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_'))
        .ok_or_else(|| format!("`{t}` needs a unit (ns, us, ms, s, min)"))?;
    let (num, unit) = t.split_at(split);
    let n: f64 = num
        .replace('_', "")
        .parse()
        .map_err(|_| format!("`{num}` is not a number"))?;
    let scale = match unit.trim() {
        "ns" => 1.0,
        "us" | "µs" => 1.0e3,
        "ms" => 1.0e6,
        "s" => 1.0e9,
        "min" => 6.0e10,
        other => return Err(format!("unknown time unit `{other}`")),
    };
    Ok((n * scale).round() as i64)
}

fn parse_value(text: &str) -> Result<Value, String> {
    let t = text.trim();
    match t {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '_'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    if num.is_empty() {
        return Ok(Value::Str(t.trim_matches('"').to_string()));
    }
    let clean = num.replace('_', "");
    let unit = unit.trim();
    if unit.is_empty() {
        if clean.contains('.') {
            return clean
                .parse::<f64>()
                .map(Value::Real)
                .map_err(|_| format!("`{t}` is not a number"));
        }
        return clean
            .parse::<i128>()
            .map(Value::Int)
            .map_err(|_| format!("`{t}` is not a number"));
    }
    let n: f64 = clean
        .parse()
        .map_err(|_| format!("`{num}` is not a number"))?;
    let u =
        neuron_sema::units::lookup_unit(unit).ok_or_else(|| format!("unknown unit `{unit}`"))?;
    let info = Rc::new(UnitInfo {
        text: unit.to_string(),
        scale: u.scale.to_f64(),
        offset: u.offset.to_f64(),
    });
    Ok(Value::Qty(info.to_si(n), info))
}

/// Parses a scenario.
pub fn parse_scenario(text: &str) -> Result<Scenario, String> {
    let mut steps = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let err = |m: String| format!("line {}: {m}", i + 1);
        let line = line.strip_prefix("at ").unwrap_or(line);
        let (time, rest) = line
            .split_once(':')
            .ok_or_else(|| err("expected `<time>: <action>`".into()))?;
        let t_ns = parse_duration(time).map_err(err)?;
        let rest = rest.trim();
        let action = if let Some(name) = rest.strip_suffix('!') {
            Action::Pulse(name.trim().to_string())
        } else if let Some(w) = rest.strip_prefix("press ") {
            Action::Press(w.trim().to_string())
        } else if let Some((name, value)) = rest.split_once('=') {
            Action::Set(name.trim().to_string(), parse_value(value).map_err(err)?)
        } else {
            return Err(err(format!("cannot read `{rest}`")));
        };
        steps.push(Step { t_ns, action });
    }
    Ok(Scenario { steps })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_read_in_nanoseconds() {
        assert_eq!(parse_duration("500 ms"), Ok(500_000_000));
        assert_eq!(parse_duration("2s"), Ok(2_000_000_000));
        assert_eq!(parse_duration("1.5 s"), Ok(1_500_000_000));
        assert!(parse_duration("5").is_err());
        assert!(parse_duration("5 parsecs").is_err());
    }

    #[test]
    fn steps_are_parsed() {
        let s = parse_scenario(
            "# demo\nat 100 ms: button = true\n2 s: sensor = 2048\n3 s: temp = 95 degC\n4 s: tick!\n5 s: press go\n",
        )
        .unwrap_or_default();
        assert_eq!(s.steps.len(), 5);
        assert_eq!(s.steps[0].t_ns, 100_000_000);
        assert!(matches!(&s.steps[3].action, Action::Pulse(n) if n == "tick"));
        assert!(matches!(&s.steps[4].action, Action::Press(n) if n == "go"));
    }

    #[test]
    fn bad_lines_are_reported_with_their_number() {
        let e = parse_scenario("1 s: x = 1\nnonsense\n")
            .err()
            .unwrap_or_default();
        assert!(e.starts_with("line 2"), "{e}");
    }
}
