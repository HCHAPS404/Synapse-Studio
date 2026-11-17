//! Clock-tree and peripheral-rate solvers (spec 07 §7.3).
//!
//! Everything here is a pure function of the chip description and the request, so the
//! results are reproducible and can be checked by property tests (CLK-1).

use crate::model::ClockSpec;

/// A way to make the system clock: `source / pre * mul / post`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSolution {
    /// Source name.
    pub source: String,
    /// Source frequency.
    pub source_hz: u64,
    /// Pre-divider (1 when there is no PLL).
    pub pre: u32,
    /// PLL multiplier (1 when there is no PLL).
    pub mul: u32,
    /// Post-divider.
    pub post: u32,
    /// Resulting frequency.
    pub hz: u64,
}

/// Why a system clock could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockError {
    /// The source does not exist or the frequency is outside its range.
    BadSource {
        /// Name asked for.
        name: String,
        /// Frequency asked for.
        hz: u64,
    },
    /// No combination gives the frequency; the nearest ones are reported.
    Unreachable {
        /// Frequency asked for.
        requested_hz: u64,
        /// Nearest achievable frequency below it.
        below: Option<u64>,
        /// Nearest achievable frequency above it.
        above: Option<u64>,
    },
}

/// Every frequency the clock tree can make from `source_hz`, within the chip limits.
pub fn candidates(spec: &ClockSpec, source: &str, source_hz: u64) -> Vec<ClockSolution> {
    let mut out = Vec::new();
    let mk = |pre: u32, mul: u32, post: u32, hz: u64| ClockSolution {
        source: source.to_string(),
        source_hz,
        pre,
        mul,
        post,
        hz,
    };
    match &spec.pll {
        None => {
            for &post in &spec.post_dividers {
                if post == 0 || source_hz % u64::from(post) != 0 {
                    continue;
                }
                let hz = source_hz / u64::from(post);
                if hz <= spec.max_hz {
                    out.push(mk(1, 1, post, hz));
                }
            }
        }
        Some(pll) => {
            for pre in pll.pre_min..=pll.pre_max {
                if pre == 0 || source_hz % u64::from(pre) != 0 {
                    continue;
                }
                let fin = source_hz / u64::from(pre);
                if fin < pll.input_min_hz || fin > pll.input_max_hz {
                    continue;
                }
                for mul in pll.mul_min..=pll.mul_max {
                    let vco = fin * u64::from(mul);
                    if vco < pll.vco_min_hz || vco > pll.vco_max_hz {
                        continue;
                    }
                    for &post in &spec.post_dividers {
                        if post == 0 || vco % u64::from(post) != 0 {
                            continue;
                        }
                        let hz = vco / u64::from(post);
                        if hz <= spec.max_hz {
                            out.push(mk(pre, mul, post, hz));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Finds a way to make `want_hz` from the named source. Among exact solutions it takes the
/// one with the smallest VCO, then the smallest pre-divider (the least power and jitter).
pub fn solve_sysclk(
    spec: &ClockSpec,
    source: &str,
    source_hz: u64,
    want_hz: u64,
) -> Result<ClockSolution, ClockError> {
    let known = spec
        .sources
        .iter()
        .find(|s| s.name == source && source_hz >= s.min_hz && source_hz <= s.max_hz);
    if known.is_none() {
        return Err(ClockError::BadSource {
            name: source.to_string(),
            hz: source_hz,
        });
    }
    let all = candidates(spec, source, source_hz);
    let mut exact: Vec<&ClockSolution> = all.iter().filter(|c| c.hz == want_hz).collect();
    exact.sort_by_key(|c| {
        (
            u64::from(c.mul) * source_hz / u64::from(c.pre),
            c.pre,
            c.post,
        )
    });
    if let Some(best) = exact.first() {
        return Ok((*best).clone());
    }
    let below = all.iter().map(|c| c.hz).filter(|&h| h < want_hz).max();
    let above = all.iter().map(|c| c.hz).filter(|&h| h > want_hz).min();
    Err(ClockError::Unreachable {
        requested_hz: want_hz,
        below,
        above,
    })
}

/// Relative error of an achieved rate, `|achieved - requested| / requested`.
pub fn rate_error(requested: f64, achieved: f64) -> f64 {
    if requested == 0.0 {
        return if achieved == 0.0 { 0.0 } else { f64::INFINITY };
    }
    (achieved - requested).abs() / requested
}

/// A UART divisor for an AVR-style asynchronous receiver.
#[derive(Debug, Clone, PartialEq)]
pub struct UartDivisor {
    /// Value of the baud-rate register (`UBRR`).
    pub ubrr: u32,
    /// Double-speed mode (`U2X`).
    pub double_speed: bool,
    /// The rate that results.
    pub achieved_baud: f64,
    /// Relative error.
    pub error: f64,
}

/// Best `UBRR` (normal or double speed) for `baud`, or `None` when it does not fit in 12 bits.
pub fn uart_avr(sysclk_hz: u64, baud: u32) -> Option<UartDivisor> {
    if baud == 0 {
        return None;
    }
    let f = sysclk_hz as f64;
    let b = f64::from(baud);
    let mut best: Option<UartDivisor> = None;
    for (per_bit, double_speed) in [(16.0, false), (8.0, true)] {
        let ubrr = (f / (per_bit * b)).round() - 1.0;
        if !(0.0..=4095.0).contains(&ubrr) {
            continue;
        }
        let ubrr = ubrr as u32;
        let achieved = f / (per_bit * (f64::from(ubrr) + 1.0));
        let cand = UartDivisor {
            ubrr,
            double_speed,
            achieved_baud: achieved,
            error: rate_error(b, achieved),
        };
        if best.as_ref().is_none_or(|x| cand.error < x.error) {
            best = Some(cand);
        }
    }
    best
}

/// A timer configured to a frequency.
#[derive(Debug, Clone, PartialEq)]
pub struct TimerSolution {
    /// Prescaler.
    pub prescaler: u32,
    /// Compare value (counts to `top` inclusive, so the period is `top + 1` counts).
    pub top: u32,
    /// The frequency that results.
    pub achieved_hz: f64,
    /// Relative error.
    pub error: f64,
}

/// A periodic tick of `want_hz` (CTC mode): the prescaler and compare value of least error.
pub fn timer_ctc(
    sysclk_hz: u64,
    prescalers: &[u32],
    bits: u32,
    want_hz: f64,
) -> Option<TimerSolution> {
    let max_top = (1u64 << bits) - 1;
    let mut best: Option<TimerSolution> = None;
    for &p in prescalers {
        if p == 0 || want_hz <= 0.0 {
            continue;
        }
        let counts = (sysclk_hz as f64 / (f64::from(p) * want_hz)).round();
        if counts < 1.0 || counts - 1.0 > max_top as f64 {
            continue;
        }
        let top = counts as u32 - 1;
        let achieved = sysclk_hz as f64 / (f64::from(p) * counts);
        let cand = TimerSolution {
            prescaler: p,
            top,
            achieved_hz: achieved,
            error: rate_error(want_hz, achieved),
        };
        if best.as_ref().is_none_or(|x| cand.error < x.error) {
            best = Some(cand);
        }
    }
    best
}

/// A fast-PWM carrier of `want_hz` with an 8-bit counter (frequency `f / (prescaler * 256)`).
pub fn pwm_fast8(sysclk_hz: u64, prescalers: &[u32], want_hz: f64) -> Option<TimerSolution> {
    let mut best: Option<TimerSolution> = None;
    for &p in prescalers {
        if p == 0 {
            continue;
        }
        let achieved = sysclk_hz as f64 / (f64::from(p) * 256.0);
        let cand = TimerSolution {
            prescaler: p,
            top: 255,
            achieved_hz: achieved,
            error: rate_error(want_hz, achieved),
        };
        if best.as_ref().is_none_or(|x| cand.error < x.error) {
            best = Some(cand);
        }
    }
    best
}

/// Claims of hardware resources, one owner each (property HW-1).
#[derive(Debug, Clone, Default)]
pub struct Owners {
    owners: std::collections::BTreeMap<String, String>,
}

impl Owners {
    /// Records that `who` owns `resource`. Returns the current owner when it is taken.
    pub fn claim(&mut self, resource: &str, who: &str) -> Result<(), String> {
        match self.owners.get(resource) {
            Some(owner) => Err(owner.clone()),
            None => {
                self.owners.insert(resource.to_string(), who.to_string());
                Ok(())
            }
        }
    }

    /// The owner of `resource`, if any.
    pub fn owner(&self, resource: &str) -> Option<&str> {
        self.owners.get(resource).map(String::as_str)
    }

    /// Number of resources owned.
    pub fn len(&self) -> usize {
        self.owners.len()
    }

    /// `true` when nothing is owned.
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }
}
