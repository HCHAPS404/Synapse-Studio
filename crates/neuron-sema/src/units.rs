//! Physical units, dimensions and exact conversions (spec 03 §3.3).
//!
//! Scales are exact rationals so that `1 s + 250 ms` is exactly `1.25 s`.
//! The only inexact constant is π (used by `deg` and `rev`), which is
//! approximated with 15 digits.

use neuron_syntax::ast::UnitExpr;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

/// An exact rational number with a positive denominator, always reduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rat {
    num: i128,
    den: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.unsigned_abs(), b.unsigned_abs());
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    i128::try_from(a).unwrap_or(1)
}

// The arithmetic methods return `Option` (overflow is an error), so they cannot
// implement the std operator traits.
#[allow(clippy::should_implement_trait)]
impl Rat {
    /// Zero.
    pub const ZERO: Rat = Rat { num: 0, den: 1 };
    /// One.
    pub const ONE: Rat = Rat { num: 1, den: 1 };

    /// Creates a reduced rational. Returns `None` for a zero denominator or overflow.
    pub fn new(num: i128, den: i128) -> Option<Rat> {
        if den == 0 {
            return None;
        }
        let g = gcd(num, den).max(1);
        let (mut n, mut d) = (num / g, den / g);
        if d < 0 {
            n = n.checked_neg()?;
            d = d.checked_neg()?;
        }
        Some(Rat { num: n, den: d })
    }

    /// An integer as a rational.
    pub fn int(n: i128) -> Rat {
        Rat { num: n, den: 1 }
    }

    /// Numerator.
    pub fn numer(&self) -> i128 {
        self.num
    }

    /// Denominator (always positive).
    pub fn denom(&self) -> i128 {
        self.den
    }

    /// `true` for zero.
    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    /// `true` when the denominator is one.
    pub fn is_integer(&self) -> bool {
        self.den == 1
    }

    /// `true` when below zero.
    pub fn is_negative(&self) -> bool {
        self.num < 0
    }

    /// Sum, or `None` on overflow.
    pub fn add(self, o: Rat) -> Option<Rat> {
        let n = self
            .num
            .checked_mul(o.den)?
            .checked_add(o.num.checked_mul(self.den)?)?;
        Rat::new(n, self.den.checked_mul(o.den)?)
    }

    /// Difference, or `None` on overflow.
    pub fn sub(self, o: Rat) -> Option<Rat> {
        self.add(Rat {
            num: o.num.checked_neg()?,
            den: o.den,
        })
    }

    /// Product, or `None` on overflow.
    pub fn mul(self, o: Rat) -> Option<Rat> {
        let g1 = gcd(self.num, o.den).max(1);
        let g2 = gcd(o.num, self.den).max(1);
        Rat::new(
            (self.num / g1).checked_mul(o.num / g2)?,
            (self.den / g2).checked_mul(o.den / g1)?,
        )
    }

    /// Quotient, or `None` on division by zero or overflow.
    pub fn div(self, o: Rat) -> Option<Rat> {
        if o.num == 0 {
            return None;
        }
        self.mul(Rat::new(o.den, o.num)?)
    }

    /// Integer power (negative exponents invert), or `None` on overflow.
    pub fn pow(self, e: i32) -> Option<Rat> {
        // Exponents this large always overflow `i128`; refuse them without looping.
        if e.unsigned_abs() > 128 {
            return None;
        }
        let base = if e < 0 {
            Rat::new(self.den, self.num)?
        } else {
            self
        };
        let mut acc = Rat::ONE;
        for _ in 0..e.unsigned_abs() {
            acc = acc.mul(base)?;
        }
        Some(acc)
    }

    /// Nearest `f64`.
    pub fn to_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Parses a Neuron numeric literal: decimal, `1e-3`, `0x2A`, `0b101`, `0o7`, with `_`.
    pub fn parse(text: &str) -> Option<Rat> {
        let t: String = text.chars().filter(|c| *c != '_').collect();
        for (prefix, radix) in [("0x", 16), ("0b", 2), ("0o", 8)] {
            if let Some(rest) = t.strip_prefix(prefix) {
                return i128::from_str_radix(rest, radix).ok().map(Rat::int);
            }
        }
        let (mant, exp) = match t.find(['e', 'E']) {
            Some(i) => (&t[..i], t[i + 1..].parse::<i32>().ok()?),
            None => (t.as_str(), 0),
        };
        if exp.abs() > 30 {
            return None;
        }
        let (int_part, frac_part) = match mant.find('.') {
            Some(i) => (&mant[..i], &mant[i + 1..]),
            None => (mant, ""),
        };
        let digits = format!("{int_part}{frac_part}");
        let n: i128 = digits.parse().ok()?;
        let frac_len = i32::try_from(frac_part.len()).ok()?;
        let e10 = exp - frac_len;
        let ten = Rat::int(10).pow(e10)?;
        Rat::int(n).mul(ten)
    }
}

impl PartialOrd for Rat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rat {
    fn cmp(&self, o: &Self) -> Ordering {
        match (self.num.checked_mul(o.den), o.num.checked_mul(self.den)) {
            (Some(a), Some(b)) => a.cmp(&b),
            _ => self
                .to_f64()
                .partial_cmp(&o.to_f64())
                .unwrap_or(Ordering::Equal),
        }
    }
}

impl fmt::Display for Rat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Exponents of the nine base dimensions.
///
/// Order: time, length, mass, current, temperature, amount, luminous intensity,
/// plane angle, count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dim(pub [i8; 9]);

const DIM_NAMES: [&str; 9] = ["s", "m", "kg", "A", "K", "mol", "cd", "rad", "count"];

impl Dim {
    /// The dimensionless dimension.
    pub const NONE: Dim = Dim([0; 9]);

    /// Dimension with a single base exponent.
    pub fn base(index: usize, exp: i8) -> Dim {
        let mut d = [0i8; 9];
        if let Some(slot) = d.get_mut(index) {
            *slot = exp;
        }
        Dim(d)
    }

    /// `true` when every exponent is zero.
    pub fn is_none(&self) -> bool {
        self.0.iter().all(|e| *e == 0)
    }

    /// Product of two dimensions, or `None` when an exponent overflows.
    pub fn mul(&self, o: &Dim) -> Option<Dim> {
        let mut d = [0i8; 9];
        for (i, slot) in d.iter_mut().enumerate() {
            *slot = self.0[i].checked_add(o.0[i])?;
        }
        Some(Dim(d))
    }

    /// Quotient of two dimensions.
    pub fn div(&self, o: &Dim) -> Option<Dim> {
        let mut d = [0i8; 9];
        for (i, slot) in d.iter_mut().enumerate() {
            *slot = self.0[i].checked_sub(o.0[i])?;
        }
        Some(Dim(d))
    }

    /// Integer power.
    pub fn pow(&self, e: i32) -> Option<Dim> {
        let mut d = [0i8; 9];
        for (i, slot) in d.iter_mut().enumerate() {
            let v = i32::from(self.0[i]).checked_mul(e)?;
            *slot = i8::try_from(v).ok()?;
        }
        Some(Dim(d))
    }

    /// Square root when every exponent is even.
    pub fn sqrt(&self) -> Option<Dim> {
        let mut d = [0i8; 9];
        for (i, slot) in d.iter_mut().enumerate() {
            if self.0[i] % 2 != 0 {
                return None;
            }
            *slot = self.0[i] / 2;
        }
        Some(Dim(d))
    }

    /// Human readable name, for example `length`, `velocity` or `m*s^-2`.
    pub fn describe(&self) -> String {
        const KNOWN: [([i8; 9], &str); 14] = [
            ([0; 9], "dimensionless"),
            ([1, 0, 0, 0, 0, 0, 0, 0, 0], "time"),
            ([0, 1, 0, 0, 0, 0, 0, 0, 0], "length"),
            ([0, 0, 1, 0, 0, 0, 0, 0, 0], "mass"),
            ([0, 0, 0, 1, 0, 0, 0, 0, 0], "electric current"),
            ([0, 0, 0, 0, 1, 0, 0, 0, 0], "temperature"),
            ([0, 0, 0, 0, 0, 0, 0, 1, 0], "angle"),
            ([0, 0, 0, 0, 0, 0, 0, 0, 1], "count"),
            ([-1, 0, 0, 0, 0, 0, 0, 0, 0], "frequency"),
            ([-1, 1, 0, 0, 0, 0, 0, 0, 0], "velocity"),
            ([-2, 1, 0, 0, 0, 0, 0, 0, 0], "acceleration"),
            ([-2, 1, 1, 0, 0, 0, 0, 0, 0], "force"),
            ([-3, 2, 1, -1, 0, 0, 0, 0, 0], "voltage"),
            ([-3, 2, 1, 0, 0, 0, 0, 0, 0], "power"),
        ];
        for (d, name) in KNOWN {
            if d == self.0 {
                return name.to_string();
            }
        }
        let mut parts = Vec::new();
        for (i, e) in self.0.iter().enumerate() {
            match *e {
                0 => {}
                1 => parts.push(DIM_NAMES[i].to_string()),
                e => parts.push(format!("{}^{e}", DIM_NAMES[i])),
            }
        }
        parts.join("*")
    }
}

/// A unit: a dimension, an exact scale and an offset to the base unit.
///
/// `value_in_base = value * scale + offset`. Only the affine temperature units
/// (`degC`, `degF`) have a non-zero offset.
#[derive(Debug, Clone)]
pub struct Unit {
    /// Dimension.
    pub dim: Dim,
    /// Multiplier to the base unit.
    pub scale: Rat,
    /// Additive offset to the base unit.
    pub offset: Rat,
    /// Name used in messages (cosmetic, ignored by equality).
    pub label: String,
}

impl PartialEq for Unit {
    fn eq(&self, o: &Self) -> bool {
        self.dim == o.dim && self.scale == o.scale && self.offset == o.offset
    }
}

impl Unit {
    /// The dimensionless unit with scale one.
    pub fn one() -> Unit {
        Unit {
            dim: Dim::NONE,
            scale: Rat::ONE,
            offset: Rat::ZERO,
            label: String::new(),
        }
    }

    fn make(dim: Dim, scale: Rat, label: &str) -> Unit {
        Unit {
            dim,
            scale,
            offset: Rat::ZERO,
            label: label.to_string(),
        }
    }

    /// `true` when the dimension is empty (ratios, `%`, plain numbers).
    pub fn is_dimensionless(&self) -> bool {
        self.dim.is_none()
    }

    /// `true` for `degC` and `degF`.
    pub fn is_affine(&self) -> bool {
        !self.offset.is_zero()
    }

    /// The same unit used as an interval (offset removed).
    pub fn interval(&self) -> Unit {
        Unit {
            offset: Rat::ZERO,
            ..self.clone()
        }
    }

    /// Product of two units (affine operands are used as intervals).
    pub fn mul(&self, o: &Unit) -> Option<Unit> {
        Some(Unit {
            dim: self.dim.mul(&o.dim)?,
            scale: self.scale.mul(o.scale)?,
            offset: Rat::ZERO,
            label: join_label(&self.label, "*", &o.label),
        })
    }

    /// Quotient of two units.
    pub fn div(&self, o: &Unit) -> Option<Unit> {
        Some(Unit {
            dim: self.dim.div(&o.dim)?,
            scale: self.scale.div(o.scale)?,
            offset: Rat::ZERO,
            label: join_label(&self.label, "/", &o.label),
        })
    }

    /// Square root, when the dimension allows it.
    pub fn sqrt(&self) -> Option<Unit> {
        // Only exact for scale 1 (the common case after normalization).
        if self.scale != Rat::ONE {
            return None;
        }
        Some(Unit {
            dim: self.dim.sqrt()?,
            scale: Rat::ONE,
            offset: Rat::ZERO,
            label: String::new(),
        })
    }

    /// Text for diagnostics: the label if there is one, otherwise the dimension.
    pub fn display(&self) -> String {
        if self.label.is_empty() {
            self.dim.describe()
        } else {
            self.label.clone()
        }
    }
}

fn join_label(a: &str, op: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => String::new(),
        (false, true) => a.to_string(),
        (true, false) if op == "/" => format!("1/{b}"),
        (true, false) => b.to_string(),
        (false, false) => format!("{a}{op}{b}"),
    }
}

/// Converts `value` from one unit to another of the same dimension.
///
/// Returns `None` when the dimensions differ or the arithmetic overflows.
pub fn convert(value: Rat, from: &Unit, to: &Unit) -> Option<Rat> {
    if from.dim != to.dim {
        return None;
    }
    let base = value.mul(from.scale)?.add(from.offset)?;
    base.sub(to.offset)?.div(to.scale)
}

/// Value of `value` expressed in the base unit of its dimension.
pub fn to_base(value: Rat, from: &Unit) -> Option<Rat> {
    value.mul(from.scale)?.add(from.offset)
}

fn r(n: i128, d: i128) -> Rat {
    Rat::new(n, d).unwrap_or(Rat::ONE)
}

/// π approximated with 15 digits (used by `deg` and `rev`).
fn pi() -> Rat {
    r(3_141_592_653_589_793, 1_000_000_000_000_000)
}

const T: usize = 0;
const L: usize = 1;
const M: usize = 2;
const I: usize = 3;
const K: usize = 4;
const N: usize = 5;
const CD: usize = 6;
const ANG: usize = 7;
const CNT: usize = 8;

fn dim(parts: &[(usize, i8)]) -> Dim {
    let mut d = Dim::NONE;
    for (i, e) in parts {
        d.0[*i] = *e;
    }
    d
}

/// Units that accept SI prefixes: `(name, dim, scale to base)`.
fn prefixable(name: &str) -> Option<(Dim, Rat)> {
    Some(match name {
        "s" => (dim(&[(T, 1)]), Rat::ONE),
        "m" => (dim(&[(L, 1)]), Rat::ONE),
        "g" => (dim(&[(M, 1)]), r(1, 1000)),
        "A" => (dim(&[(I, 1)]), Rat::ONE),
        "Hz" => (dim(&[(T, -1)]), Rat::ONE),
        "N" => (dim(&[(M, 1), (L, 1), (T, -2)]), Rat::ONE),
        "Pa" => (dim(&[(M, 1), (L, -1), (T, -2)]), Rat::ONE),
        "bar" => (dim(&[(M, 1), (L, -1), (T, -2)]), Rat::int(100_000)),
        "J" => (dim(&[(M, 1), (L, 2), (T, -2)]), Rat::ONE),
        "W" => (dim(&[(M, 1), (L, 2), (T, -3)]), Rat::ONE),
        "V" => (dim(&[(M, 1), (L, 2), (T, -3), (I, -1)]), Rat::ONE),
        "Ohm" => (dim(&[(M, 1), (L, 2), (T, -3), (I, -2)]), Rat::ONE),
        "F" => (dim(&[(M, -1), (L, -2), (T, 4), (I, 2)]), Rat::ONE),
        "H" => (dim(&[(M, 1), (L, 2), (T, -2), (I, -2)]), Rat::ONE),
        "C" => (dim(&[(T, 1), (I, 1)]), Rat::ONE),
        "B" => (dim(&[(CNT, 1)]), Rat::ONE),
        "Bd" => (dim(&[(T, -1)]), Rat::ONE),
        "L" => (dim(&[(L, 3)]), r(1, 1000)),
        "mol" => (dim(&[(N, 1)]), Rat::ONE),
        "rad" => (dim(&[(ANG, 1)]), Rat::ONE),
        "lx" => (dim(&[(CD, 1), (L, -2)]), Rat::ONE),
        _ => return None,
    })
}

fn prefix_scale(c: char) -> Option<Rat> {
    Some(match c {
        'n' => r(1, 1_000_000_000),
        'u' => r(1, 1_000_000),
        'm' => r(1, 1000),
        'c' => r(1, 100),
        'k' => Rat::int(1000),
        'M' => Rat::int(1_000_000),
        'G' => Rat::int(1_000_000_000),
        _ => return None,
    })
}

fn normalize(name: &str) -> String {
    name.replace("°C", "degC")
        .replace("°F", "degF")
        .replace('°', "deg")
        .replace('Ω', "Ohm")
        .replace('µ', "u")
}

/// Looks up a built-in unit by name, with SI prefixes and Unicode aliases.
///
/// ```
/// use neuron_sema::units::{convert, lookup_unit, Rat};
/// let ms = lookup_unit("ms").unwrap();
/// let s = lookup_unit("s").unwrap();
/// assert_eq!(convert(Rat::int(250), &ms, &s), Rat::new(1, 4));
/// ```
pub fn lookup_unit(name: &str) -> Option<Unit> {
    let n = normalize(name);
    let n = n.as_str();
    let exact = match n {
        "%" => Some((Dim::NONE, r(1, 100), Rat::ZERO)),
        "ppm" => Some((Dim::NONE, r(1, 1_000_000), Rat::ZERO)),
        "min" => Some((dim(&[(T, 1)]), Rat::int(60), Rat::ZERO)),
        "h" => Some((dim(&[(T, 1)]), Rat::int(3600), Rat::ZERO)),
        "rpm" => Some((dim(&[(T, -1)]), r(1, 60), Rat::ZERO)),
        "in" => Some((dim(&[(L, 1)]), r(254, 10_000), Rat::ZERO)),
        "kg" => Some((dim(&[(M, 1)]), Rat::ONE, Rat::ZERO)),
        "psi" => Some((
            dim(&[(M, 1), (L, -1), (T, -2)]),
            r(6_894_757_293_168, 1_000_000_000),
            Rat::ZERO,
        )),
        "K" => Some((dim(&[(K, 1)]), Rat::ONE, Rat::ZERO)),
        "degC" => Some((dim(&[(K, 1)]), Rat::ONE, r(5463, 20))),
        "degF" => Some((dim(&[(K, 1)]), r(5, 9), r(45_967, 180))),
        "deg" => Some((
            dim(&[(ANG, 1)]),
            pi().div(Rat::int(180)).unwrap_or(Rat::ONE),
            Rat::ZERO,
        )),
        "rev" => Some((
            dim(&[(ANG, 1)]),
            pi().mul(Rat::int(2)).unwrap_or(Rat::ONE),
            Rat::ZERO,
        )),
        "cd" => Some((dim(&[(CD, 1)]), Rat::ONE, Rat::ZERO)),
        "count" => Some((dim(&[(CNT, 1)]), Rat::ONE, Rat::ZERO)),
        "1" => Some((Dim::NONE, Rat::ONE, Rat::ZERO)),
        _ => None,
    };
    if let Some((d, s, o)) = exact {
        return Some(Unit {
            dim: d,
            scale: s,
            offset: o,
            label: name.to_string(),
        });
    }
    if let Some((d, s)) = prefixable(n) {
        return Some(Unit::make(d, s, name));
    }
    let mut chars = n.chars();
    let first = chars.next()?;
    let rest = chars.as_str();
    let p = prefix_scale(first)?;
    let (d, s) = prefixable(rest)?;
    Some(Unit::make(d, p.mul(s)?, name))
}

/// Units defined by the program with `unit name = expr`.
#[derive(Debug, Default, Clone)]
pub struct UnitTable {
    user: BTreeMap<String, Unit>,
}

impl UnitTable {
    /// Empty table (only built-in units).
    pub fn new() -> Self {
        Self::default()
    }

    /// Defines a program unit.
    pub fn define(&mut self, name: &str, unit: Unit) {
        let mut u = unit;
        u.label = name.to_string();
        self.user.insert(name.to_string(), u);
    }

    /// Looks up one unit name (program units shadow built-ins).
    pub fn get(&self, name: &str) -> Option<Unit> {
        self.user.get(name).cloned().or_else(|| lookup_unit(name))
    }

    /// Evaluates a unit expression. On failure returns the first unknown unit name.
    ///
    /// An affine unit (`degC`) is kept as is only when it is the sole factor with
    /// exponent one; inside compound units it is used as an interval.
    pub fn resolve(&self, e: &UnitExpr) -> Result<Unit, String> {
        let single = e.factors.len() == 1 && e.factors[0].1 == 1;
        let mut acc = Unit::one();
        let mut pos = Vec::new();
        let mut neg = Vec::new();
        for (id, exp) in &e.factors {
            let u = self.get(&id.name).ok_or_else(|| id.name.clone())?;
            if *exp >= 0 {
                pos.push((id.name.clone(), *exp));
            } else {
                neg.push((id.name.clone(), -*exp));
            }
            if single {
                return Ok(u);
            }
            let u = u.interval();
            let scale = u.scale.pow(*exp).ok_or_else(|| id.name.clone())?;
            let d = u.dim.pow(*exp).ok_or_else(|| id.name.clone())?;
            acc.dim = acc.dim.mul(&d).ok_or_else(|| id.name.clone())?;
            acc.scale = acc.scale.mul(scale).ok_or_else(|| id.name.clone())?;
        }
        acc.label = format_label(&pos, &neg);
        Ok(acc)
    }
}

fn format_label(pos: &[(String, i32)], neg: &[(String, i32)]) -> String {
    let f = |v: &[(String, i32)]| {
        v.iter()
            .map(|(n, e)| {
                if *e == 1 {
                    n.clone()
                } else {
                    format!("{n}^{e}")
                }
            })
            .collect::<Vec<_>>()
            .join("*")
    };
    match (pos.is_empty(), neg.is_empty()) {
        (_, true) => f(pos),
        (true, false) => format!("1/{}", f(neg)),
        (false, false) => format!("{}/{}", f(pos), f(neg)),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn u(n: &str) -> Unit {
        lookup_unit(n).expect("known unit")
    }

    #[test]
    fn prefixes_and_aliases() {
        assert_eq!(u("ms").scale, r(1, 1000));
        assert_eq!(u("µs"), u("us"));
        assert_eq!(u("kOhm").scale, Rat::int(1000));
        assert_eq!(u("kg").scale, Rat::ONE);
        assert_eq!(u("g").scale, r(1, 1000));
        assert_eq!(u("mm").dim, u("m").dim);
        assert_eq!(u("°C"), u("degC"));
        assert!(lookup_unit("flurb").is_none());
        assert!(lookup_unit("min").is_some());
    }

    #[test]
    fn exact_second_arithmetic() {
        let s = u("s");
        let ms = u("ms");
        let sum = convert(Rat::int(250), &ms, &s)
            .and_then(|x| x.add(Rat::ONE))
            .expect("no overflow");
        assert_eq!(sum, r(5, 4));
    }

    #[test]
    fn temperature_conversions() {
        let c = u("degC");
        let f = u("degF");
        let k = u("K");
        assert_eq!(convert(Rat::int(0), &c, &k), Some(r(5463, 20)));
        assert_eq!(convert(Rat::int(100), &c, &f), Some(Rat::int(212)));
        assert_eq!(convert(Rat::int(32), &f, &c), Some(Rat::ZERO));
    }

    #[test]
    fn dimension_mismatch_is_none() {
        assert!(convert(Rat::ONE, &u("m"), &u("s")).is_none());
    }

    #[test]
    fn parses_literals() {
        assert_eq!(Rat::parse("1_000"), Some(Rat::int(1000)));
        assert_eq!(Rat::parse("0x2A"), Some(Rat::int(42)));
        assert_eq!(Rat::parse("2.5e3"), Some(Rat::int(2500)));
        assert_eq!(Rat::parse("1e-3"), Some(r(1, 1000)));
        assert_eq!(Rat::parse("0.5"), Some(r(1, 2)));
    }

    #[test]
    fn velocity_dimension() {
        let mm = u("mm");
        let s = u("s");
        let v = mm.div(&s).expect("ok");
        assert_eq!(v.dim.describe(), "velocity");
        assert_eq!(v.scale, r(1, 1000));
    }

    const LINEAR: [&str; 8] = ["s", "ms", "us", "min", "h", "ns", "kHz", "m"];

    fn same_dim(a: &str, b: &str) -> bool {
        u(a).dim == u(b).dim
    }

    proptest! {
        /// Round trip between two units of the same dimension is exact.
        #[test]
        fn conversion_round_trip(v in -1_000_000i64..1_000_000, i in 0usize..8, j in 0usize..8) {
            let (a, b) = (u(LINEAR[i]), u(LINEAR[j]));
            prop_assume!(a.dim == b.dim);
            let x = Rat::int(i128::from(v));
            let there = convert(x, &a, &b).expect("convertible");
            let back = convert(there, &b, &a).expect("convertible");
            prop_assert_eq!(back, x);
        }

        /// UNIT-1 for addition: adding in declared units equals adding in base units.
        #[test]
        fn unit1_addition(a in -100_000i64..100_000, b in -100_000i64..100_000, i in 0usize..8, j in 0usize..8) {
            prop_assume!(same_dim(LINEAR[i], LINEAR[j]));
            let (ua, ub) = (u(LINEAR[i]), u(LINEAR[j]));
            let (xa, xb) = (Rat::int(i128::from(a)), Rat::int(i128::from(b)));
            // In declared units: convert b to a's unit, add, convert result to base.
            let in_a = convert(xb, &ub, &ua).and_then(|y| y.add(xa)).expect("ok");
            let declared = to_base(in_a, &ua).expect("ok");
            // In base units: convert both first.
            let base = to_base(xa, &ua).and_then(|p| to_base(xb, &ub).and_then(|q| p.add(q))).expect("ok");
            prop_assert_eq!(declared, base);
        }

        /// UNIT-1 for multiplication: scales multiply exactly.
        #[test]
        fn unit1_multiplication(a in -1000i64..1000, b in -1000i64..1000, i in 0usize..8, j in 0usize..8) {
            let (ua, ub) = (u(LINEAR[i]), u(LINEAR[j]));
            let prod = ua.mul(&ub).expect("ok");
            let (xa, xb) = (Rat::int(i128::from(a)), Rat::int(i128::from(b)));
            let declared = xa.mul(xb).and_then(|p| p.mul(prod.scale)).expect("ok");
            let base = to_base(xa, &ua).and_then(|p| to_base(xb, &ub).and_then(|q| p.mul(q))).expect("ok");
            prop_assert_eq!(declared, base);
        }

        /// UNIT-1 for affine units: a temperature difference does not depend on the scale used.
        #[test]
        fn unit1_affine_difference(a in -300i64..300, b in -300i64..300) {
            let c = u("degC");
            let k = u("K");
            let (xa, xb) = (Rat::int(i128::from(a)), Rat::int(i128::from(b)));
            let diff_c = xa.sub(xb).expect("ok");
            let diff_via_k = to_base(xa, &c).and_then(|p| to_base(xb, &c).and_then(|q| p.sub(q))).expect("ok");
            prop_assert_eq!(diff_c, diff_via_k);
            prop_assert_eq!(convert(diff_c, &c.interval(), &k.interval()), Some(diff_c));
        }
    }
}
