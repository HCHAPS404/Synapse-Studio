//! Function blocks written into the output for the `std.control` blocks that IEC 61131-3
//! does not have. Each one computes exactly what the reference interpreter computes.

use super::gen::Style;

struct Fb {
    name: &'static str,
    inputs: &'static [(&'static str, &'static str)],
    outputs: &'static [(&'static str, &'static str)],
    locals: &'static [(&'static str, &'static str)],
    body: &'static str,
}

const FBS: &[Fb] = &[
    Fb {
        name: "NEURON_HYSTERESIS",
        inputs: &[("X", "LREAL"), ("LOW", "LREAL"), ("HIGH", "LREAL")],
        outputs: &[("Q", "BOOL")],
        locals: &[],
        body: "IF $X >= $HIGH THEN\n    $Q := TRUE;\nELSIF $X <= $LOW THEN\n    $Q := FALSE;\nEND_IF;",
    },
    Fb {
        name: "NEURON_DEBOUNCE",
        inputs: &[("X", "BOOL"), ("T", "TIME"), ("NOW", "TIME")],
        outputs: &[("Q", "BOOL")],
        locals: &[("SINCE", "TIME"), ("WAITING", "BOOL")],
        body: "IF $X = $Q THEN\n    $WAITING := FALSE;\nELSIF NOT $WAITING THEN\n    $WAITING := TRUE;\n    $SINCE := $NOW;\nELSIF $NOW - $SINCE >= $T THEN\n    $Q := $X;\n    $WAITING := FALSE;\nEND_IF;",
    },
    Fb {
        name: "NEURON_RAMP",
        inputs: &[("X", "LREAL"), ("RATE", "LREAL"), ("DT", "LREAL")],
        outputs: &[("OUT", "LREAL")],
        locals: &[("STEP", "LREAL")],
        body: "$STEP := ABS($RATE) * $DT;\n$OUT := $OUT + LIMIT(-$STEP, $X - $OUT, $STEP);",
    },
    Fb {
        name: "NEURON_LOWPASS",
        inputs: &[("X", "LREAL"), ("CUTOFF", "LREAL"), ("DT", "LREAL")],
        outputs: &[("OUT", "LREAL")],
        locals: &[("STARTED", "BOOL"), ("RC", "LREAL"), ("A", "LREAL")],
        body: "IF $CUTOFF > 0.0 THEN\n    $RC := 1.0 / (6.283185307179586 * $CUTOFF);\nELSE\n    $RC := 0.0;\nEND_IF;\nIF $RC + $DT > 0.0 THEN\n    $A := $DT / ($RC + $DT);\nELSE\n    $A := 1.0;\nEND_IF;\nIF NOT $STARTED THEN\n    $OUT := $X;\n    $STARTED := TRUE;\nELSE\n    $OUT := $OUT + $A * ($X - $OUT);\nEND_IF;",
    },
    Fb {
        name: "NEURON_HIGHPASS",
        inputs: &[("X", "LREAL"), ("CUTOFF", "LREAL"), ("DT", "LREAL")],
        outputs: &[("OUT", "LREAL")],
        locals: &[("STARTED", "BOOL"), ("PX", "LREAL"), ("RC", "LREAL"), ("A", "LREAL")],
        body: "IF $CUTOFF > 0.0 THEN\n    $RC := 1.0 / (6.283185307179586 * $CUTOFF);\nELSE\n    $RC := 0.0;\nEND_IF;\nIF $RC + $DT > 0.0 THEN\n    $A := $DT / ($RC + $DT);\nELSE\n    $A := 1.0;\nEND_IF;\nIF NOT $STARTED THEN\n    $OUT := 0.0;\n    $STARTED := TRUE;\nELSE\n    $OUT := (1.0 - $A) * ($OUT + $X - $PX);\nEND_IF;\n$PX := $X;",
    },
    Fb {
        name: "NEURON_MOVING_AVERAGE",
        inputs: &[("X", "LREAL"), ("WINDOW", "INT")],
        outputs: &[("OUT", "LREAL")],
        locals: &[("BUF", "ARRAY [0..63] OF LREAL"), ("N", "INT"), ("W", "INT"), ("I", "INT"), ("SUM", "LREAL")],
        body: "IF $WINDOW < 1 THEN\n    $W := 1;\nELSIF $WINDOW > 64 THEN\n    $W := 64;\nELSE\n    $W := $WINDOW;\nEND_IF;\nIF $N < $W THEN\n    $BUF[$N] := $X;\n    $N := $N + 1;\nELSE\n    FOR $I := 0 TO $W - 2 DO\n        $BUF[$I] := $BUF[$I + 1];\n    END_FOR;\n    $BUF[$W - 1] := $X;\nEND_IF;\n$SUM := 0.0;\nFOR $I := 0 TO $N - 1 DO\n    $SUM := $SUM + $BUF[$I];\nEND_FOR;\n$OUT := $SUM / TO_LREAL($N);",
    },
    Fb {
        name: "NEURON_PID",
        inputs: &[
            ("E", "LREAL"),
            ("KP", "LREAL"),
            ("KI", "LREAL"),
            ("KD", "LREAL"),
            ("LO", "LREAL"),
            ("HI", "LREAL"),
            ("ANTIWINDUP", "BOOL"),
            ("DT", "LREAL"),
        ],
        outputs: &[("OUT", "LREAL")],
        locals: &[("INTEGRAL", "LREAL"), ("LAST", "LREAL"), ("RAW", "LREAL"), ("D", "LREAL")],
        body: "IF $DT > 0.0 THEN\n    $D := $KD * ($E - $LAST) / $DT;\nELSE\n    $D := 0.0;\nEND_IF;\n$RAW := $KP * $E + $INTEGRAL + $D;\n$OUT := LIMIT($LO, $RAW, $HI);\nIF NOT ($ANTIWINDUP AND ABS($RAW - $OUT) > 0.0) THEN\n    $INTEGRAL := $INTEGRAL + $KI * $E * $DT;\nEND_IF;\n$LAST := $E;",
    },
];

/// The source of the helper function block `name` in the given style.
pub(crate) fn helper(name: &str, style: Style, unit: &str) -> String {
    let Some(fb) = FBS.iter().find(|f| f.name == name) else {
        return String::new();
    };
    let prefix = if style == Style::Scl { "#" } else { "" };
    let mut out = String::new();
    match style {
        Style::Iec => out.push_str(&format!("FUNCTION_BLOCK {}\n", fb.name)),
        Style::Scl => out.push_str(&format!(
            "FUNCTION_BLOCK \"{}\"\n{{ S7_Optimized_Access := 'TRUE' }}\nVERSION : 0.1\n",
            fb.name
        )),
    }
    let section = |out: &mut String, kw: &str, vars: &[(&str, &str)]| {
        if vars.is_empty() {
            return;
        }
        out.push_str(&format!("{unit}{kw}\n"));
        for (n, t) in vars {
            out.push_str(&format!("{unit}{unit}{n} : {t};\n"));
        }
        out.push_str(&format!("{unit}END_VAR\n"));
    };
    section(&mut out, "VAR_INPUT", fb.inputs);
    section(&mut out, "VAR_OUTPUT", fb.outputs);
    section(&mut out, "VAR", fb.locals);
    if style == Style::Scl {
        out.push_str("BEGIN\n");
    }
    for line in fb.body.lines() {
        out.push_str(unit);
        out.push_str(&line.replace("    ", unit).replace('$', prefix));
        out.push('\n');
    }
    out.push_str("END_FUNCTION_BLOCK\n");
    out
}
