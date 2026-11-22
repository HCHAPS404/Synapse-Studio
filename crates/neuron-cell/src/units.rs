//! Quantities in manifests (`2 ms`, `100 MBd`), checked with the unit table of the analyzer.
use neuron_sema::units::{lookup_unit, to_base, Dim, Rat};

fn split(text: &str) -> Result<(Rat, &str), String> {
    let t = text.trim();
    let at = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .ok_or_else(|| format!("`{t}` needs a unit"))?;
    let (num, unit) = t.split_at(at);
    if num.is_empty() {
        return Err(format!("`{t}` does not start with a number"));
    }
    let n = Rat::parse(num).ok_or_else(|| format!("`{num}` is not a number"))?;
    Ok((n, unit.trim()))
}

/// Value of `text` in the base unit times `per`, as a whole number.
fn whole(text: &str, dim: Dim, what: &str, per: i128) -> Result<u64, String> {
    let (n, unit) = split(text)?;
    let u = lookup_unit(unit).ok_or_else(|| format!("unknown unit `{unit}` in `{text}`"))?;
    if u.dim != dim {
        return Err(format!("`{text}` is not {what}"));
    }
    let overflow = || format!("`{text}` is out of range");
    let v = to_base(n, &u)
        .and_then(|b| b.mul(Rat::int(per)))
        .ok_or_else(overflow)?;
    if v.is_negative() {
        return Err(format!("`{text}` is negative"));
    }
    if !v.is_integer() {
        return Err(format!("`{text}` is not a whole number of base steps"));
    }
    u64::try_from(v.numer()).map_err(|_| overflow())
}

/// A duration such as `500 us` in nanoseconds.
///
/// ```
/// assert_eq!(neuron_cell::duration_ns("2 ms"), Ok(2_000_000));
/// assert!(neuron_cell::duration_ns("2 V").is_err());
/// ```
pub fn duration_ns(text: &str) -> Result<u64, String> {
    whole(text, Dim::base(0, 1), "a duration", 1_000_000_000)
}

/// A bit rate such as `100 MBd` in baud (symbols per second).
///
/// ```
/// assert_eq!(neuron_cell::rate_bd("115200 Bd"), Ok(115_200));
/// ```
pub fn rate_bd(text: &str) -> Result<u64, String> {
    whole(text, Dim::base(0, -1), "a rate", 1)
}
