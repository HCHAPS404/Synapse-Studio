//! Electrical compatibility of wired links (task D4.2, `NE0801`).
//!
//! Only media of the `zero_hop` family join two devices electrically; network and bus
//! media (Modbus TCP, S7) leave the I/O domains apart. For a wired link the output domain
//! of the sender (its I/O family with a current limit) must equal the input domain of
//! the receiver (its family with a `sink`/`source` kind, else its first family with a
//! voltage), unless the medium is isolated or the link uses one of the medium's
//! isolating or level-shifting options (`zero_hop(gpio, opto_24v)`).
use crate::{CellDiag, Family, Library, LinkReport, Manifest};

/// Two voltage domains closer than this are the same domain.
const SAME_VOLTS: f64 = 0.1;

pub(crate) fn output_volts(m: &Manifest) -> Option<f64> {
    m.io.values()
        .find(|f| f.max_ma.is_some() && f.volts.is_some())
        .or_else(|| m.io.values().find(|f| f.volts.is_some()))
        .and_then(|f| f.volts)
}

pub(crate) fn input_volts(m: &Manifest) -> Option<f64> {
    m.io.values()
        .find(|f| f.kind.is_some() && f.volts.is_some())
        .or_else(|| m.io.values().find(|f| f.volts.is_some()))
        .and_then(|f| f.volts)
}

/// `NE0801` for every wired link of `rows` that joins two voltage domains without
/// isolation or level shifting. `file` is the cell file and `instance_manifest` maps a
/// device instance to its manifest name.
pub fn check_electrical(
    rows: &[LinkReport],
    file: &str,
    instance_manifest: &dyn Fn(&str) -> Option<String>,
    lib: &Library,
) -> Vec<CellDiag> {
    let mut diags = Vec::new();
    for row in rows {
        let Some(medium) = lib.medium(&row.medium) else {
            continue;
        };
        if medium.family != Family::ZeroHop || medium.isolated {
            continue;
        }
        if row.options.iter().any(|o| medium.isolating.contains(o)) {
            continue;
        }
        let man = |inst: &str| instance_manifest(inst).and_then(|n| lib.device(&n));
        let (Some(s), Some(r)) = (man(&row.sender), man(&row.receiver)) else {
            continue;
        };
        let (Some(out_v), Some(in_v)) = (output_volts(s), input_volts(r)) else {
            continue;
        };
        if (out_v - in_v).abs() <= SAME_VOLTS {
            continue;
        }
        let mut d = CellDiag::new(
            "NE0801",
            file,
            Some(row.span),
            format!(
                "link `{} -> {}` wires a {out_v} V output of `{}` ({}) to a {in_v} V input of `{}` ({})",
                row.from, row.to, row.sender, s.name, row.receiver, r.name
            ),
        );
        if medium.isolating.is_empty() {
            d = d.help("add a level shifter or an isolated stage, or use a network medium");
        } else {
            d = d.help(format!(
                "write `via {}({})` to add an isolating or level-shifting stage",
                medium.name,
                medium
                    .options
                    .iter()
                    .filter(|o| !medium.isolating.contains(o))
                    .chain(medium.isolating.first())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        diags.push(d);
    }
    diags
}
