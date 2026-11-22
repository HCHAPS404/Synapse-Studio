//! Hooks for running several interpreters on one clock (the cell co-simulation).
//!
//! A cell simulation steps one [`Interp`] per device to the same instant, reads what a
//! sender produced and writes it into the receiver between steps. These methods are
//! the whole outside interface that needs: peek the next scheduled instant, read a
//! named value, write a named value.

use crate::value::Value;
use crate::Interp;
use neuron_nir::ItemDef;

/// Makes `v` fit the variant of the value it replaces: a bool into an integer, an
/// integer into a bool or a real, a real into an integer.
fn coerce(v: Value, like: Option<&Value>) -> Value {
    match (&v, like) {
        (Value::Bool(b), Some(Value::Int(_))) => Value::Int(i128::from(*b)),
        (Value::Bool(b), Some(Value::Real(_))) => Value::Real(if *b { 1.0 } else { 0.0 }),
        (Value::Int(i), Some(Value::Bool(_))) => Value::Bool(*i != 0),
        (Value::Int(i), Some(Value::Real(_))) => Value::Real(*i as f64),
        (Value::Real(r), Some(Value::Bool(_))) => Value::Bool(*r != 0.0),
        (Value::Real(r), Some(Value::Int(_))) => Value::Int(*r as i128),
        _ => v,
    }
}

impl Interp {
    /// The next instant at which something is scheduled, in nanoseconds.
    ///
    /// ```
    /// use neuron_interp::{Interp, Options};
    /// use neuron_sema::{analyze, SemaOptions, SourceFile};
    /// let src = "module m for mcu\nio led: gpio.output @ D13\ntask t every 5 ms:\n  led.toggle()\nsafety:\n  safe_state:\n    led.write(false)\n";
    /// let a = analyze(vec![SourceFile { path: "m.neuron".into(), text: src.into() }], None, &SemaOptions::default());
    /// let it = Interp::new(a.nir.unwrap(), Options::default()).unwrap();
    /// assert_eq!(it.next_event_ns(), Some(0));
    /// ```
    pub fn next_event_ns(&self) -> Option<i64> {
        self.queue.keys().next().map(|k| k.0)
    }

    /// Moves the clock to `t_ns` without running anything scheduled at or after it, so
    /// that a [`write_named`](Interp::write_named) is stamped `t_ns` and is seen by the
    /// activities released at `t_ns`. Does nothing when `t_ns` is not later than now.
    /// Callers step to the next scheduled instant, so nothing earlier is skipped.
    pub fn advance_clock(&mut self, t_ns: i64) {
        if t_ns > self.now {
            self.now = t_ns;
        }
    }

    /// Reads a named value: a signal (committed value), a module or PLC variable, or an
    /// `io` binding (last written output, else last input). The first match in module
    /// order wins. `None` when nothing has that name.
    pub fn read_named(&self, name: &str) -> Option<Value> {
        if let Some((id, _)) = self.sig_info.iter().find(|(_, i)| i.name == name) {
            return self.signals.get(id).cloned();
        }
        if let Some(id) = self.var_id(name) {
            return self.globals.get(&id).cloned();
        }
        self.io
            .iter()
            .find(|(_, s)| s.name == name)
            .and_then(|(id, s)| s.value.clone().or_else(|| self.inputs.get(id).cloned()))
    }

    /// Writes a named value from outside, at the current instant. A signal is committed
    /// (and traced if it changed), an `io` or PLC input is applied like a scenario `set`
    /// (events such as `.rising` are raised), a variable is assigned. The value is
    /// coerced to the kind of the one it replaces. Returns `false` when nothing has
    /// that name.
    pub fn write_named(&mut self, name: &str, v: Value) -> bool {
        let sig = self
            .sig_info
            .iter()
            .find(|(_, i)| i.name == name)
            .map(|(id, _)| id.clone());
        if let Some(id) = sig {
            let v = coerce(v, self.signals.get(&id));
            self.commit_signal(&id, v);
            return true;
        }
        if let Some(keys) = self.idx.inputs_by_name.get(name).cloned() {
            for k in keys {
                let like = self.inputs.get(&k).or_else(|| self.globals.get(&k));
                let v = coerce(v.clone(), like);
                self.apply_input(&k, v);
            }
            return true;
        }
        if let Some(id) = self.var_id(name) {
            let v = coerce(v, self.globals.get(&id));
            self.globals.insert(id, v);
            return true;
        }
        false
    }

    /// The item id of a module-level or PLC variable with this name.
    fn var_id(&self, name: &str) -> Option<String> {
        for module in &self.nir.modules {
            for item in &module.items {
                match &item.def {
                    ItemDef::Var { var } if var.name == name => return Some(var.id.clone()),
                    ItemDef::PlcSection { vars, .. } => {
                        if let Some(v) = vars.iter().find(|v| v.name == name) {
                            return Some(v.id.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::Options;
    use neuron_sema::{analyze, SemaOptions, SourceFile};

    fn load(src: &str) -> Interp {
        let a = analyze(
            vec![SourceFile {
                path: "m.neuron".into(),
                text: src.into(),
            }],
            None,
            &SemaOptions::default(),
        );
        let Some(nir) = a.nir.clone() else {
            panic!("{}", a.render())
        };
        Interp::new(nir, Options::default()).unwrap()
    }

    #[test]
    fn a_signal_written_from_outside_is_read_back_and_traced() {
        let mut it = load(
            "module m for mcu\nsignal level: u16\nio led: gpio.output @ D13\ntask t every 1 ms:\n  led.write(level > 3)\nsafety:\n  safe_state:\n    led.write(false)\n",
        );
        assert_eq!(it.next_event_ns(), Some(0));
        assert!(it.write_named("level", Value::Int(7)));
        assert_eq!(it.read_named("level").map(|v| v.show()), Some("7".into()));
        it.run_until(2_000_000);
        assert_eq!(it.read_named("led").map(|v| v.show()), Some("true".into()));
        it.run_until(2_000_000);
        it.advance_clock(5_000_000);
        assert!(it.write_named("level", Value::Int(0)));
        assert_eq!(it.trace().last().map(|e| e.t_ns), Some(5_000_000));
        assert!(!it.write_named("nope", Value::Int(1)));
        assert!(it.read_named("nope").is_none());
    }

    #[test]
    fn a_plc_input_and_a_variable_take_values_between_steps() {
        let mut it = load(
            "module p for plc\nscan 10 ms\ninputs:\n  go: bool @ %I0.0\noutputs:\n  run: bool @ %Q0.0\nmemory:\n  held: u16\nrung r:\n  run = go\nsafety:\n  safe_state:\n    run = false\n",
        );
        it.run_until(10_000_000);
        assert!(it.write_named("go", Value::Bool(true)));
        assert!(it.write_named("held", Value::Bool(true)));
        assert_eq!(it.read_named("held").map(|v| v.show()), Some("1".into()));
        it.run_until(30_000_000);
        assert_eq!(it.read_named("run").map(|v| v.show()), Some("true".into()));
    }
}
