//! Task C2: the IEC 61131-3 emitters, checked by running their output.
//!
//! Every `plc` program of the corpus is emitted as Structured Text and scanned in the
//! virtual PLC of `neuron-st` with random inputs. With scan-counted timers the output
//! trace must equal the reference interpreter's; with IEC wall-clock timers it must stay
//! within the bound written in the fidelity report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_emit::{emitter, Class, EmitOptions, Package, Policy};
use neuron_interp::{Action, Interp, Options, Scenario, Step, TraceKind, Value as IV};
use neuron_nir::{ItemDef, Nir, Type};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_st::ast::IntTy;
use neuron_st::{parse, Clock, Plc, Value as SV};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn programs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = root().join("examples/corpus/plc");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    files.sort();
    files.push(root().join("examples/conveyor_plc.neuron"));
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        out.push((
            name,
            std::fs::read_to_string(&f).unwrap().replace("\r\n", "\n"),
        ));
    }
    out
}

fn nir_of(name: &str, text: &str) -> Nir {
    let a = analyze(
        vec![SourceFile {
            path: name.into(),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    a.nir
        .clone()
        .unwrap_or_else(|| panic!("{name}:\n{}", a.render()))
}

#[derive(Clone, Debug)]
enum In {
    B(bool),
    I(i64),
}

fn inputs(nir: &Nir) -> Vec<(String, bool)> {
    let mut v = Vec::new();
    for item in &nir.modules[0].items {
        if let ItemDef::PlcSection { section, vars } = &item.def {
            if section == "inputs" {
                for x in vars {
                    v.push((x.name.clone(), matches!(x.vty, Type::Bool)));
                }
            }
        }
    }
    v
}

fn scan_ns(nir: &Nir) -> i64 {
    for item in &nir.modules[0].items {
        if let ItemDef::Scan { period } = &item.def {
            return neuron_emit::nirx::dur_ns(period).unwrap();
        }
    }
    10_000_000
}

/// Random inputs, each changing 3 ms after a scan instant so the order is never ambiguous.
fn schedule(nir: &Nir, seed: u64, end: i64) -> Vec<(i64, String, In)> {
    let scan = scan_ns(nir);
    let mut rng = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let mut next = || {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        rng >> 33
    };
    let ins = inputs(nir);
    let mut state: Vec<bool> = vec![false; ins.len()];
    let mut out = Vec::new();
    let mut t = 3_000_000;
    // Seed 5 is a pulse pattern: the first input pulses often, the others seldom, so
    // counters reach their preset and timers expire.
    let pulses = seed == 5;
    while t < end {
        if pulses {
            for (k, (name, is_bool)) in ins.iter().enumerate() {
                let odds = if k == 0 { 3 } else { 60 };
                if *is_bool && next() % odds == 0 {
                    out.push((t, name.clone(), In::B(true)));
                    out.push((t + scan * 2, name.clone(), In::B(false)));
                }
            }
            t += scan * 4;
            continue;
        }
        for (k, (name, is_bool)) in ins.iter().enumerate() {
            if *is_bool {
                if next() % 9 == 0 {
                    state[k] = !state[k];
                    out.push((t, name.clone(), In::B(state[k])));
                }
            } else if next() % 15 == 0 {
                out.push((t, name.clone(), In::I(700 + (next() % 300) as i64)));
            }
        }
        // Now and then the inputs stay still for about three seconds, so that timers fire.
        let pause = if next() % 25 == 0 {
            300
        } else {
            1 + (next() % 4) as i64
        };
        t += scan * pause;
    }
    out
}

type Obs = Vec<(i64, String, String)>;

fn run_reference(nir: &Nir, sched: &[(i64, String, In)], end: i64) -> Obs {
    let mut it = Interp::new(
        nir.clone(),
        Options {
            end_ns: end,
            ..Options::default()
        },
    )
    .unwrap();
    let steps = sched
        .iter()
        .map(|(t, n, v)| Step {
            t_ns: *t,
            action: Action::Set(
                n.clone(),
                match v {
                    In::B(b) => IV::Bool(*b),
                    In::I(i) => IV::Int(i128::from(*i)),
                },
            ),
        })
        .collect();
    it.load_scenario(&Scenario { steps });
    it.run();
    it.trace()
        .iter()
        .filter(|e| e.kind == TraceKind::Output)
        .map(|e| (e.t_ns, e.name.clone(), e.value.clone()))
        .collect()
}

fn st_source(pkg: &Package) -> String {
    let mut parts: Vec<(&String, &String)> = pkg
        .files
        .iter()
        .filter(|(n, _)| n.ends_with(".st"))
        .collect();
    parts.sort_by_key(|(n, _)| match n.as_str() {
        "NeuronLib.st" => 0,
        "GlobalVars.st" => 1,
        "config.st" => 3,
        _ => 2,
    });
    parts
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_plc(
    src: &str,
    program: Option<&str>,
    scan: i64,
    sched: &[(i64, String, In)],
    end: i64,
    clock: Clock,
) -> Obs {
    let unit = parse(src).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let mut plc = Plc::new(unit, program, scan, clock).unwrap();
    let ins: Vec<(i64, String, SV)> = sched
        .iter()
        .map(|(t, n, v)| {
            (
                *t,
                n.clone(),
                match v {
                    In::B(b) => SV::Bool(*b),
                    In::I(i) => SV::Int(
                        *i,
                        IntTy {
                            bits: 32,
                            signed: true,
                        },
                    ),
                },
            )
        })
        .collect();
    plc.run(end, &ins).unwrap_or_else(|e| panic!("{e}\n{src}"));
    plc.trace()
        .iter()
        .map(|c| (c.t_ns, c.name.clone(), c.value.show()))
        .collect()
}

fn has(nir: &Nir, f: impl Fn(&ItemDef) -> bool) -> bool {
    nir.modules[0].items.iter().any(|i| f(&i.def))
}

const END: i64 = 8_000_000_000;

fn cut(o: Obs, end: i64) -> Obs {
    o.into_iter().filter(|(t, _, _)| *t < end).collect()
}

#[test]
fn structured_text_matches_the_interpreter_exactly_with_scan_counted_timers() {
    let st = emitter("st").unwrap();
    let mut checked = 0;
    for (name, text) in programs() {
        let nir = nir_of(&name, &text);
        if has(&nir, |d| {
            matches!(d, ItemDef::Sequence { .. } | ItemDef::Fsm { .. })
        }) {
            continue;
        }
        let pkg = st
            .emit(&nir, &EmitOptions::default())
            .unwrap_or_else(|d| panic!("{name}: {d:?}"));
        let src = st_source(&pkg);
        let scan = scan_ns(&nir);
        let mut most = 0;
        for seed in 1..=5 {
            let sched = schedule(&nir, seed, END);
            let want = cut(run_reference(&nir, &sched, END), END - 2 * scan);
            most = most.max(want.len());
            let got = cut(
                run_plc(&src, None, scan, &sched, END, Clock::ScanCounted),
                END - 2 * scan,
            );
            if got != want {
                let k = got
                    .iter()
                    .zip(&want)
                    .position(|(a, b)| a != b)
                    .unwrap_or(got.len().min(want.len()));
                let dump = root().join(format!("out/c2-{name}.st"));
                let _ = std::fs::write(&dump, &src);
                panic!(
                    "{name} seed {seed}: first difference at change {k}: PLC {:?}, reference {:?} ({} vs {} changes); ST written to {}",
                    got.get(k),
                    want.get(k),
                    got.len(),
                    want.len(),
                    dump.display()
                );
            }
        }
        // At least one output change after the first publication of the outputs.
        assert!(most >= 2, "{name}: no seed exercised the program");
        checked += 1;
    }
    assert!(checked >= 9, "only {checked} programs checked");
}

/// Same values in the same order, each change within `bound` of the reference.
fn within(got: &Obs, want: &Obs, bound: i64) -> Result<(), String> {
    let names: std::collections::BTreeSet<&String> = want.iter().map(|(_, n, _)| n).collect();
    for n in names {
        let a: Vec<_> = got.iter().filter(|x| &x.1 == n).collect();
        let b: Vec<_> = want.iter().filter(|x| &x.1 == n).collect();
        let k = a.len().min(b.len());
        if a.len().abs_diff(b.len()) > 1 {
            return Err(format!(
                "`{n}`: {} changes in the PLC, {} in the reference",
                a.len(),
                b.len()
            ));
        }
        for i in 0..k {
            if a[i].2 != b[i].2 {
                return Err(format!("`{n}` change {i}: {} vs {}", a[i].2, b[i].2));
            }
            if (a[i].0 - b[i].0).abs() > bound {
                return Err(format!(
                    "`{n}` change {i}: at {} ns vs {} ns",
                    a[i].0, b[i].0
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn wall_clock_timers_and_sequences_stay_within_the_reported_bound() {
    let st = emitter("st").unwrap();
    for (name, text) in programs() {
        // A flasher is a loop of timers: each cycle is longer on the PLC (checked below).
        if name.starts_with("p04") {
            continue;
        }
        let nir = nir_of(&name, &text);
        let pkg = st.emit(&nir, &EmitOptions::default()).unwrap();
        let src = st_source(&pkg);
        let scan = scan_ns(&nir);
        let seq = has(&nir, |d| {
            matches!(d, ItemDef::Sequence { .. } | ItemDef::Fsm { .. })
        });
        let bound = if seq { 2 * scan } else { scan };
        if seq {
            let entry = pkg
                .fidelity
                .entries
                .iter()
                .find(|e| e.construct.starts_with("sequence"))
                .unwrap();
            assert!(entry.note.contains("two scans"), "{}", entry.note);
        }
        for seed in 1..=4 {
            let sched = schedule(&nir, seed, END);
            let want = cut(run_reference(&nir, &sched, END), END - 4 * scan);
            let clock = if seq {
                Clock::ScanCounted
            } else {
                Clock::WallClock
            };
            let got = cut(
                run_plc(&src, None, scan, &sched, END, clock),
                END - 4 * scan,
            );
            if let Err(e) = within(&got, &want, bound) {
                panic!("{name} seed {seed}: {e}\n{src}");
            }
        }
    }
}

#[test]
fn a_flasher_cycle_is_one_scan_longer_per_timer_on_a_wall_clock_plc() {
    let (name, text) = programs()
        .into_iter()
        .find(|(n, _)| n.starts_with("p04"))
        .unwrap();
    let nir = nir_of(&name, &text);
    let src = st_source(
        &emitter("st")
            .unwrap()
            .emit(&nir, &EmitOptions::default())
            .unwrap(),
    );
    let sched = vec![(3_000_000, "alarm".to_string(), In::B(true))];
    let period = |o: &Obs| {
        let on: Vec<i64> = o
            .iter()
            .filter(|x| x.1 == "lamp" && x.2 == "true")
            .map(|x| x.0)
            .collect();
        on[2] - on[1]
    };
    let neuron = period(&run_reference(&nir, &sched, 5_000_000_000));
    let plc = period(&run_plc(
        &src,
        None,
        10_000_000,
        &sched,
        5_000_000_000,
        Clock::WallClock,
    ));
    // 500 ms + 500 ms, plus one scan for each rung that passes a timer output on.
    assert_eq!(neuron, 1_020_000_000);
    assert_eq!(plc, neuron + 2 * 10_000_000, "two timers, one scan each");
}

#[test]
fn every_timer_is_reported_as_equivalent_and_the_policy_can_refuse_it() {
    let (name, text) = programs()
        .into_iter()
        .find(|(n, _)| n.starts_with("conveyor"))
        .unwrap();
    let nir = nir_of(&name, &text);
    let pkg = emitter("st")
        .unwrap()
        .emit(&nir, &EmitOptions::default())
        .unwrap();
    let timers: Vec<_> = pkg
        .fidelity
        .entries
        .iter()
        .filter(|e| e.construct.starts_with("timer"))
        .collect();
    assert_eq!(timers.len(), 1);
    assert_eq!(timers[0].class, Class::Equivalent);
    assert!(pkg.fidelity.summary.exact > 3);
    assert!(Policy {
        worst: Class::Equivalent
    }
    .check(&pkg.fidelity)
    .is_empty());
    let refused = Policy {
        worst: Class::Exact,
    }
    .check(&pkg.fidelity);
    assert_eq!(refused.len(), pkg.fidelity.entries.len());
    assert!(refused.iter().all(|d| d.code == "NE1003"));
    let json = pkg.fidelity.to_json();
    assert!(json.contains("\"summary\"") && json.ends_with("}\n"));
}

#[test]
fn the_output_is_deterministic_and_maps_back_to_the_source() {
    for (name, text) in programs() {
        let nir = nir_of(&name, &text);
        for e in ["st", "scl", "plcopen"] {
            let a = emitter(e)
                .unwrap()
                .emit(&nir, &EmitOptions::default())
                .unwrap();
            let b = emitter(e)
                .unwrap()
                .emit(&nir, &EmitOptions::default())
                .unwrap();
            assert_eq!(a.all_files(), b.all_files(), "{name} {e}");
        }
        let st = emitter("st")
            .unwrap()
            .emit(&nir, &EmitOptions::default())
            .unwrap();
        let map = st.maps.values().next().unwrap();
        assert!(!map.mappings.is_empty());
        let file = st.files.get(&map.generated).unwrap();
        let lines: Vec<&str> = file.lines().collect();
        for m in &map.mappings {
            assert!(
                m.gen[0] >= 1 && m.gen[2] <= lines.len(),
                "{name}: {:?}",
                m.gen
            );
            assert!(m.src.span[1] <= text.len());
        }
        assert!(map.mappings.iter().any(|m| m
            .view
            .as_deref()
            .is_some_and(|v| v.starts_with("rung:") || v.starts_with("sequence:"))));
    }
}

#[test]
fn the_yaskawa_profile_moves_the_io_to_a_global_variable_list() {
    let (name, text) = programs()
        .into_iter()
        .find(|(n, _)| n.starts_with("p01"))
        .unwrap();
    let nir = nir_of(&name, &text);
    let opts = EmitOptions {
        profile: "yaskawa".into(),
        ..EmitOptions::default()
    };
    let pkg = emitter("st").unwrap().emit(&nir, &opts).unwrap();
    assert!(pkg.files["GlobalVars.st"].contains("start AT %IX0.0 : BOOL;"));
    assert!(!pkg.files.contains_key("config.st"));
    let src = st_source(&pkg);
    let sched = schedule(&nir, 7, END);
    let want = cut(run_reference(&nir, &sched, END), END - 20_000_000);
    let got = cut(
        run_plc(&src, None, 10_000_000, &sched, END, Clock::ScanCounted),
        END - 20_000_000,
    );
    assert_eq!(got, want);
    let bad = EmitOptions {
        profile: "nope".into(),
        ..EmitOptions::default()
    };
    assert!(emitter("st").unwrap().emit(&nir, &bad).is_err());
}

#[test]
fn scl_runs_like_the_structured_text() {
    let scl = emitter("scl").unwrap();
    for (name, text) in programs() {
        let nir = nir_of(&name, &text);
        if has(&nir, |d| {
            matches!(d, ItemDef::Sequence { .. } | ItemDef::Fsm { .. })
        }) {
            continue;
        }
        let pkg = scl.emit(&nir, &EmitOptions::default()).unwrap();
        let module = &nir.modules[0];
        let prog = neuron_st::parse(
            &pkg.files
                .iter()
                .find(|(n, _)| n.ends_with(".scl") && !n.starts_with("NeuronLib"))
                .unwrap()
                .1
                .clone(),
        );
        let fb = prog.unwrap_or_else(|e| panic!("{name}: {e}"));
        let fb_name = fb.pous[0].name.clone();
        // A program that wires the function block to addressed I/O, as the cyclic OB does.
        let mut wrap = String::new();
        if let Some(lib) = pkg.files.get("NeuronLib.scl") {
            wrap.push_str(lib);
        }
        wrap.push_str(
            pkg.files
                .iter()
                .find(|(n, _)| n.ends_with(".scl") && !n.starts_with("NeuronLib"))
                .unwrap()
                .1,
        );
        let mut decl = String::new();
        let mut args = Vec::new();
        let mut outs = Vec::new();
        for item in &module.items {
            if let ItemDef::PlcSection { section, vars } = &item.def {
                for v in vars {
                    let ty = if matches!(v.vty, Type::Bool) {
                        "BOOL"
                    } else {
                        "INT"
                    };
                    let at = match (section.as_str(), &v.at) {
                        ("inputs", _) => format!(" AT %IX9.{}", args.len()),
                        ("outputs", _) => format!(" AT %QX9.{}", outs.len()),
                        _ => String::new(),
                    };
                    if section == "memory" {
                        continue;
                    }
                    decl.push_str(&format!("{}{at} : {ty};\n", v.name));
                    if section == "inputs" {
                        args.push(format!("{0} := {0}", v.name));
                    } else {
                        outs.push(format!("{0} := inst.{0};", v.name));
                    }
                }
            }
        }
        wrap.push_str(&format!(
            "\nPROGRAM wrap\nVAR\n{decl}inst : \"{fb_name}\";\nEND_VAR\ninst({});\n{}\nEND_PROGRAM\n",
            args.join(", "),
            outs.join("\n")
        ));
        let scan = scan_ns(&nir);
        let sched = schedule(&nir, 3, END);
        let want = cut(run_reference(&nir, &sched, END), END - 2 * scan);
        let got = cut(
            run_plc(&wrap, Some("wrap"), scan, &sched, END, Clock::ScanCounted),
            END - 2 * scan,
        );
        assert_eq!(got, want, "{name}\n{wrap}");
        assert!(pkg.files["PLC_tags.csv"].starts_with("Name,Data Type,Logical Address"));
    }
}

#[test]
fn plcopen_xml_is_well_formed_and_uses_ld_and_sfc() {
    let px = emitter("plcopen").unwrap();
    let mut ld = 0;
    let mut sfc = 0;
    for (name, text) in programs() {
        let nir = nir_of(&name, &text);
        let pkg = px.emit(&nir, &EmitOptions::default()).unwrap();
        let (file, xml) = pkg.files.iter().next().unwrap();
        assert!(file.ends_with(".xml"));
        let doc = roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{name}: {e}\n{xml}"));
        let root = doc.root_element();
        assert_eq!(root.tag_name().name(), "project");
        assert_eq!(
            root.tag_name().namespace(),
            Some("http://www.plcopen.org/xml/tc6_0201")
        );
        if doc.descendants().any(|n| n.has_tag_name("LD")) {
            ld += 1;
        }
        if doc.descendants().any(|n| n.has_tag_name("SFC")) {
            sfc += 1;
        }
        // Every connection refers to an element of the same body.
        for body in doc.descendants().filter(|n| n.has_tag_name("body")) {
            let ids: std::collections::BTreeSet<&str> = body
                .descendants()
                .filter_map(|n| n.attribute("localId"))
                .collect();
            for c in body.descendants().filter(|n| n.has_tag_name("connection")) {
                assert!(
                    ids.contains(c.attribute("refLocalId").unwrap()),
                    "{name}: dangling connection"
                );
            }
        }
    }
    assert!(ld >= 5, "{ld} LD bodies");
    assert!(sfc >= 1, "{sfc} SFC bodies");
}

#[test]
fn a_construct_without_a_translation_is_ne0610() {
    let src = "module m for plc\nscan 10 ms\n\ninputs:\n  a: bool @ %I0.0\n\noutputs:\n  q: bool @ %Q0.0\n\nfn twice(x: bool) -> bool = x\n\nrung r:\n  q = twice(a)\n\nsafety:\n  safe_state:\n    q = false\n";
    let nir = nir_of("m.neuron", src);
    let d = emitter("st")
        .unwrap()
        .emit(&nir, &EmitOptions::default())
        .unwrap_err();
    assert!(d.iter().all(|x| x.code == "NE0610"));
    assert!(d[0].message.contains("call"), "{}", d[0].message);
    let host = nir_of("h.neuron", "module h for host\nfn f() -> i32 = 1\n");
    assert!(emitter("st")
        .unwrap()
        .emit(&host, &EmitOptions::default())
        .is_err());
}
