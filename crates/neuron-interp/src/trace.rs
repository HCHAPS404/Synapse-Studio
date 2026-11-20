//! The observation of a run: what the program did, in time order.

/// Kind of a trace entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceKind {
    /// An `io` output or PLC output changed.
    Output,
    /// A `signal` became visible (at its deadline).
    Signal,
    /// A fault was raised.
    Fault,
    /// The safety logic reset a fault.
    Reset,
    /// An event was raised or an asynchronous action changed state.
    Event,
    /// A state machine changed state.
    State,
    /// Text printed by the program.
    Log,
    /// A robot motion started. `name` is `ptp`, `linear` or `arc`; `value` is the
    /// resolved target as text: `joints(0, -30, 60, 0, 60, 0) deg` for a joint target,
    /// `pallet.xyz(80, 60, 50) mm` for a point in a frame (lengths in mm, angles in deg,
    /// numbers with at most six decimals and no trailing zeros). An `arc` motion appends
    /// ` via <point>`. Rendered as `MOVE  linear = pallet.xyz(80, 60, 50) mm`.
    Motion,
}

impl TraceKind {
    /// Short tag used in the text rendering.
    pub fn tag(self) -> &'static str {
        match self {
            TraceKind::Output => "OUT",
            TraceKind::Signal => "SIG",
            TraceKind::Fault => "FAULT",
            TraceKind::Reset => "RESET",
            TraceKind::Event => "EVENT",
            TraceKind::State => "STATE",
            TraceKind::Log => "LOG",
            TraceKind::Motion => "MOVE",
        }
    }
}

/// One observation.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceEvent {
    /// Instant in nanoseconds since the start of the run.
    pub t_ns: i64,
    /// Kind.
    pub kind: TraceKind,
    /// Name of the output, signal, state machine, fault or event.
    pub name: String,
    /// Value as text (empty when there is none).
    pub value: String,
}

impl TraceEvent {
    /// One line of text: `   500.000 ms  OUT    led = true`.
    pub fn render(&self) -> String {
        let t = fmt_time(self.t_ns);
        if self.value.is_empty() {
            format!("{t:>14}  {:<5}  {}", self.kind.tag(), self.name)
        } else {
            format!(
                "{t:>14}  {:<5}  {} = {}",
                self.kind.tag(),
                self.name,
                self.value
            )
        }
    }
}

/// Formats nanoseconds as milliseconds with three decimals.
pub fn fmt_time(ns: i64) -> String {
    format!("{} ms", fmt_f64_fixed(ns as f64 / 1.0e6))
}

fn fmt_f64_fixed(x: f64) -> String {
    format!("{x:.3}")
}

/// Renders a whole trace, one line per entry.
pub fn render(trace: &[TraceEvent]) -> String {
    let mut s = String::new();
    for e in trace {
        s.push_str(&e.render());
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_have_time_kind_and_value() {
        let e = TraceEvent {
            t_ns: 500_000_000,
            kind: TraceKind::Output,
            name: "led".into(),
            value: "true".into(),
        };
        assert_eq!(e.render(), "    500.000 ms  OUT    led = true");
    }
}
