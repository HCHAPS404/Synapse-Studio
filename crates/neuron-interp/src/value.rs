//! Runtime values of the interpreter.
//!
//! Numbers with a unit are stored in base SI units (with the affine offset applied for
//! `degC` and `degF`), together with the unit they are displayed in. Plain integers keep
//! their exact value; integer overflow is checked against the type of the expression.

use neuron_nir as nir;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// How a quantity is shown: the unit text and its exact factor and offset to base units.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitInfo {
    /// Unit text (`mm/s`, `degC`).
    pub text: String,
    /// Factor to the base unit.
    pub scale: f64,
    /// Offset to the base unit (non-zero for `degC` and `degF`).
    pub offset: f64,
}

impl UnitInfo {
    /// Builds the display information from a resolved NIR unit.
    pub fn from_nir(u: &nir::Unit) -> UnitInfo {
        let scale = parse_num(&u.scale);
        UnitInfo {
            text: u.text.clone(),
            scale: if scale == 0.0 { 1.0 } else { scale },
            offset: parse_num(&u.offset),
        }
    }

    /// Converts a number written in this unit to base units.
    pub fn to_si(&self, n: f64) -> f64 {
        n * self.scale + self.offset
    }

    /// Converts a value in base units to a number in this unit.
    pub fn from_si(&self, si: f64) -> f64 {
        (si - self.offset) / self.scale
    }
}

/// Parses an exact number as written in NIR: a decimal or `n/d`. Empty text is zero.
pub fn parse_num(s: &str) -> f64 {
    if let Some((n, d)) = s.split_once('/') {
        let n = n.trim().parse::<f64>().unwrap_or(0.0);
        let d = d.trim().parse::<f64>().unwrap_or(1.0);
        if d == 0.0 {
            0.0
        } else {
            n / d
        }
    } else {
        s.trim().parse::<f64>().unwrap_or(0.0)
    }
}

/// A user struct, class instance or native function block.
#[derive(Debug)]
pub struct Obj {
    /// Item id of the definition (empty for native blocks).
    pub def: String,
    /// Type name (`LowPass`, `ton`).
    pub name: String,
    /// `true` for blocks implemented by the interpreter.
    pub native: bool,
    /// Fields in declaration order.
    pub fields: Vec<(String, Value)>,
}

impl Obj {
    /// Reads a field.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// Writes a field, adding it when it does not exist.
    pub fn set(&mut self, name: &str, v: Value) {
        if let Some(slot) = self.fields.iter_mut().find(|(k, _)| k == name) {
            slot.1 = v;
        } else {
            self.fields.push((name.to_string(), v));
        }
    }
}

/// An enum value.
#[derive(Debug)]
pub struct Variant {
    /// Item id of the enum.
    pub enum_id: String,
    /// Enum name.
    pub enum_name: String,
    /// Variant name.
    pub name: String,
    /// Payload values.
    pub payload: Vec<Value>,
}

/// A lambda with the values it captured.
#[derive(Debug)]
pub struct Closure {
    /// Parameter names and local ids.
    pub params: Vec<(String, String)>,
    /// Body.
    pub body: nir::Expr,
    /// Locals visible when the lambda was created.
    pub captured: BTreeMap<String, Value>,
}

/// A runtime value.
#[derive(Debug, Clone)]
pub enum Value {
    /// No value.
    Unit,
    /// Boolean.
    Bool(bool),
    /// Integer (also bit vectors).
    Int(i128),
    /// Real number without a unit.
    Real(f64),
    /// Quantity in base units, with the unit it is displayed in.
    Qty(f64, Rc<UnitInfo>),
    /// Text.
    Str(String),
    /// Tuple (also vectors such as `xyz(1, 2, 3)`).
    Tuple(Vec<Value>),
    /// Array.
    Array(Vec<Value>),
    /// `a..b` or `a..=b`.
    Range(Box<Value>, Box<Value>, bool),
    /// `a -> b`.
    Map(Box<Value>, Box<Value>),
    /// Struct, class instance or native block (shared; copied by [`Value::deep`]).
    Obj(Rc<RefCell<Obj>>),
    /// Enum value.
    Variant(Rc<Variant>),
    /// Optional value.
    Opt(Option<Box<Value>>),
    /// Result value.
    Res(Result<Box<Value>, Box<Value>>),
    /// Handle of an asynchronous action.
    Handle(usize),
    /// An `io` binding, by item id.
    Io(String),
    /// A function, by item id.
    Func(String),
    /// A lambda.
    Lambda(Rc<Closure>),
    /// A struct, class or enum used as a value, by item id.
    Type(String),
    /// A package module, by id.
    Module(String),
    /// A state machine, by item id.
    Fsm(String),
    /// A name from outside the package (a device, a pin, a geometry value).
    Opaque(String),
}

impl Value {
    /// Numeric value in base units, when the value is a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Real(r) => Some(*r),
            Value::Qty(si, _) => Some(*si),
            _ => None,
        }
    }

    /// Integer value, when the value is an integer.
    pub fn as_int(&self) -> Option<i128> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Truth value used by conditions. Anything that is not a boolean or a non-zero
    /// integer is false (for example a device input that was never driven).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            _ => false,
        }
    }

    /// Copy that does not share structs or class instances with the original.
    pub fn deep(&self) -> Value {
        match self {
            Value::Obj(rc) => {
                let o = rc.borrow();
                Value::Obj(Rc::new(RefCell::new(Obj {
                    def: o.def.clone(),
                    name: o.name.clone(),
                    native: o.native,
                    fields: o
                        .fields
                        .iter()
                        .map(|(k, v)| (k.clone(), v.deep()))
                        .collect(),
                })))
            }
            Value::Tuple(v) => Value::Tuple(v.iter().map(Value::deep).collect()),
            Value::Array(v) => Value::Array(v.iter().map(Value::deep).collect()),
            Value::Opt(Some(b)) => Value::Opt(Some(Box::new(b.deep()))),
            other => other.clone(),
        }
    }

    /// Structural equality. Numbers compare by value in base units.
    pub fn same(&self, o: &Value) -> bool {
        match (self, o) {
            (Value::Unit, Value::Unit) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) | (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same(y))
            }
            (Value::Variant(a), Value::Variant(b)) => {
                a.enum_id == b.enum_id
                    && a.name == b.name
                    && a.payload.len() == b.payload.len()
                    && a.payload.iter().zip(&b.payload).all(|(x, y)| x.same(y))
            }
            (Value::Opt(a), Value::Opt(b)) => match (a, b) {
                (None, None) => true,
                (Some(x), Some(y)) => x.same(y),
                _ => false,
            },
            (Value::Res(a), Value::Res(b)) => match (a, b) {
                (Ok(x), Ok(y)) | (Err(x), Err(y)) => x.same(y),
                _ => false,
            },
            (Value::Obj(a), Value::Obj(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                let (a, b) = (a.borrow(), b.borrow());
                a.name == b.name
                    && a.fields.len() == b.fields.len()
                    && a.fields
                        .iter()
                        .zip(&b.fields)
                        .all(|((ka, va), (kb, vb))| ka == kb && va.same(vb))
            }
            (Value::Opaque(a), Value::Opaque(b)) => a == b,
            (Value::Io(a), Value::Io(b)) => a == b,
            (a, b) => match (a.as_f64(), b.as_f64()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            },
        }
    }

    /// Text used in traces and by `print`.
    pub fn show(&self) -> String {
        match self {
            Value::Unit => "()".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Real(r) => fmt_f64(*r),
            Value::Qty(si, u) => format!("{} {}", fmt_f64(u.from_si(*si)), u.text),
            Value::Str(s) => s.clone(),
            Value::Tuple(v) => format!("({})", join(v)),
            Value::Array(v) => format!("[{}]", join(v)),
            Value::Range(a, b, inc) => {
                format!(
                    "{}{}{}",
                    a.show(),
                    if *inc { "..=" } else { ".." },
                    b.show()
                )
            }
            Value::Map(a, b) => format!("{} -> {}", a.show(), b.show()),
            Value::Obj(rc) => {
                let o = rc.borrow();
                let f: Vec<String> = o
                    .fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.show()))
                    .collect();
                format!("{}({})", o.name, f.join(", "))
            }
            Value::Variant(v) => {
                if v.payload.is_empty() {
                    format!("{}.{}", v.enum_name, v.name)
                } else {
                    format!("{}.{}({})", v.enum_name, v.name, join(&v.payload))
                }
            }
            Value::Opt(None) => "none".to_string(),
            Value::Opt(Some(b)) => b.show(),
            Value::Res(Ok(b)) => format!("Ok({})", b.show()),
            Value::Res(Err(b)) => format!("Err({})", b.show()),
            Value::Handle(h) => format!("handle#{h}"),
            Value::Io(id)
            | Value::Func(id)
            | Value::Type(id)
            | Value::Module(id)
            | Value::Fsm(id) => id.clone(),
            Value::Lambda(_) => "<lambda>".to_string(),
            Value::Opaque(s) => s.clone(),
        }
    }
}

fn join(v: &[Value]) -> String {
    v.iter().map(Value::show).collect::<Vec<_>>().join(", ")
}

/// Formats a real number with at most six decimals and no trailing zeros.
pub fn fmt_f64(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_formatted_without_noise() {
        assert_eq!(fmt_f64(0.0), "0");
        assert_eq!(fmt_f64(62.5), "62.5");
        assert_eq!(fmt_f64(0.1 + 0.2), "0.3");
        assert_eq!(fmt_f64(-0.0), "0");
        assert_eq!(fmt_f64(3.0), "3");
    }

    #[test]
    fn exact_text_is_parsed() {
        assert_eq!(parse_num("0.001"), 0.001);
        assert_eq!(parse_num("1/4"), 0.25);
        assert_eq!(parse_num(""), 0.0);
    }

    #[test]
    fn quantities_show_their_unit() {
        let u = Rc::new(UnitInfo {
            text: "degC".into(),
            scale: 1.0,
            offset: 273.15,
        });
        let v = Value::Qty(333.15, u);
        assert_eq!(v.show(), "60 degC");
    }

    #[test]
    fn deep_copies_do_not_share_objects() {
        let o = Value::Obj(Rc::new(RefCell::new(Obj {
            def: String::new(),
            name: "P".into(),
            native: false,
            fields: vec![("x".into(), Value::Int(1))],
        })));
        let c = o.deep();
        if let Value::Obj(rc) = &o {
            rc.borrow_mut().set("x", Value::Int(2));
        }
        assert!(!o.same(&c));
    }
}
