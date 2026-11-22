//! Task D5.1: the technical dossier of a cell.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    dossier, fmt_ns, link_table, partition, Cut, Dossier, DossierOptions, Drive, DriveValue,
    Library, SimOptions, SAFETY_WORDING,
};
use neuron_nir::Nir;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn file(rel: &str) -> SourceFile {
    SourceFile {
        path: Path::new(rel)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        text: std::fs::read_to_string(root().join(rel)).unwrap(),
    }
}

/// The five-class cell of D2.2, extended with a wired link and a Modbus link.
fn lab() -> Vec<SourceFile> {
    vec![
        file("examples/corpus/plc/p01_start_stop.neuron"),
        file("examples/corpus/mcu/m01_blink.neuron"),
        file("examples/corpus/robot/r01_home_and_wave.neuron"),
        file("examples/corpus/hdl/d01_counter.neuron"),
        file("examples/corpus/host/h01_gateway_counter.neuron"),
        SourceFile {
            path: "lab.neuron".into(),
            text: "cell Lab\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  device board: arduino_uno_r3 @ usb\n  device arm: generic_6axis @ ip(192.168.0.20)\n  device fpga: generic_artix7_board @ usb\n  device pi: raspberry_pi_5 @ ip(192.168.0.30)\n  place p01_start_stop on plc\n  place m01_blink on board\n  place r01_home_and_wave on arm\n  place d01_counter on fpga\n  place h01_gateway_counter on pi\n  link fpga.done -> plc.start via zero_hop(gpio, opto_24v) within 15 ms\n  link plc.motor -> arm.go via modbus_tcp every 10 ms within 25 ms\n".into(),
        },
    ]
}

fn bottling() -> Vec<SourceFile> {
    let dir = root().join("examples");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| {
            file(&format!(
                "examples/{}",
                f.file_name().unwrap().to_string_lossy()
            ))
        })
        .collect()
}

fn opts() -> DossierOptions {
    DossierOptions {
        sim: Some(SimOptions {
            end_ns: 200_000_000,
            ..SimOptions::default()
        }),
    }
}

/// A run with stimuli (the motor starts, so link 1 carries a value) and a cut of link 0.
fn lab_opts() -> DossierOptions {
    let drive = |t_ns: u64, name: &str| Drive {
        t_ns,
        device: "plc".into(),
        name: name.into(),
        value: DriveValue::Bool(true),
    };
    DossierOptions {
        sim: Some(SimOptions {
            end_ns: 300_000_000,
            drives: vec![drive(0, "stop"), drive(10_000_000, "start")],
            cuts: vec![Cut {
                link: 0,
                at_ns: 100_000_000,
            }],
            ..SimOptions::default()
        }),
    }
}

fn analyzed(sources: &[SourceFile]) -> (Nir, Library) {
    let a = analyze(sources.to_vec(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    (a.nir.clone().unwrap(), Library::builtin().unwrap())
}

fn make(sources: &[SourceFile]) -> Dossier {
    let (nir, lib) = analyzed(sources);
    let plan = partition(&nir, &lib).unwrap().remove(0);
    dossier(&plan, &nir, sources, &lib, &opts())
}

/// The cells of the table that follows the heading `section`, one vector per row.
fn table_rows(md: &str, section: &str) -> Vec<Vec<String>> {
    let mut lines = md.lines().skip_while(|l| !l.starts_with(section));
    lines.next();
    lines
        .skip_while(|l| !l.starts_with('|'))
        .skip(2)
        .take_while(|l| l.starts_with('|'))
        .map(|l| {
            l.trim_start_matches("| ")
                .trim_end_matches(" |")
                .split(" | ")
                .map(str::to_string)
                .collect()
        })
        .collect()
}

#[test]
fn the_lab_dossier_is_complete() {
    let d = make(&lab());
    assert_eq!(d.cell, "Lab");
    assert!(
        d.build_status.iter().all(|s| s.state == "built"),
        "{:?}",
        d.build_status
    );
    assert_eq!(d.build_status.len(), 5);
    for h in [
        "## 1. Summary",
        "## 2. Build status",
        "## 3. Wiring list",
        "## 4. I/O list",
        "## 5. Pin and address tables",
        "## 6. Link table and budgets",
        "### Timing diagram",
        "## 7. Bill of materials",
        "## 8. Safety report",
        "## 9. Test report (co-simulation)",
    ] {
        assert!(d.markdown.contains(h), "missing {h}");
    }
    // Wiring: the one wired link, with its voltage domains and isolation.
    let wiring = table_rows(&d.markdown, "## 3.");
    assert_eq!(wiring.len(), 1, "{wiring:?}");
    assert_eq!(wiring[0][1], "fpga.done");
    assert!(
        wiring[0][5].ends_with(" V") && wiring[0][6].ends_with(" V"),
        "{wiring:?}"
    );
    assert!(wiring[0][7].contains("opto_24v"));
    // Register map and Wire/pin tables carry the Modbus link.
    assert!(d.markdown.contains("HR100"));
    // BOM: one row per manifest.
    let bom = table_rows(&d.markdown, "## 7.");
    assert_eq!(bom.len(), 5, "{bom:?}");
    assert!(bom.iter().all(|r| r[3] == "1"));
    // Test report: both links ran their devices and the table has a row per link.
    let tests = table_rows(&d.markdown, "## 9.");
    assert_eq!(tests.len(), 5);
    assert!(d.markdown.contains("Simulated time: 200 ms"));
    assert!(d.html.contains("<svg "));
}

/// `NEURON_DOSSIER_OUT=dir cargo test -p neuron-cell --test d5_dossier` writes the
/// dossiers of the two cells for a visual check.
#[test]
fn the_dossiers_can_be_written_for_inspection() {
    let Ok(dir) = std::env::var("NEURON_DOSSIER_OUT") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    for (name, sources) in [("lab", lab()), ("bottling", bottling())] {
        let d = make(&sources);
        std::fs::write(Path::new(&dir).join(format!("{name}.md")), &d.markdown).unwrap();
        std::fs::write(Path::new(&dir).join(format!("{name}.html")), &d.html).unwrap();
    }
}

#[test]
fn the_same_inputs_give_identical_bytes() {
    for sources in [lab(), bottling()] {
        let (a, b) = (make(&sources), make(&sources));
        assert_eq!(a.markdown, b.markdown);
        assert_eq!(a.html, b.html);
        assert_eq!(a, b);
    }
}

#[test]
fn the_link_table_section_equals_the_link_table() {
    for sources in [lab(), bottling()] {
        let (nir, lib) = analyzed(&sources);
        let plan = partition(&nir, &lib).unwrap().remove(0);
        let d = dossier(&plan, &nir, &sources, &lib, &opts());
        let reports = link_table(&nir, &plan, &lib).unwrap();
        assert!(!reports.is_empty());
        let rows = table_rows(&d.markdown, "## 6.");
        assert_eq!(rows.len(), reports.len());
        let opt = |v: Option<u64>| v.map_or("-".to_string(), fmt_ns);
        for (i, (row, r)) in rows.iter().zip(&reports).enumerate() {
            let options = if r.options.is_empty() {
                "-".to_string()
            } else {
                r.options.join(", ")
            };
            let expected = vec![
                format!("l{i}"),
                r.from.clone(),
                r.to.clone(),
                r.medium.clone(),
                options,
                opt(r.every_ns),
                fmt_ns(r.latency.serialize_ns),
                fmt_ns(r.latency.transport_ns),
                fmt_ns(r.latency.receiver_phase_ns),
                fmt_ns(r.latency.polling_ns),
                fmt_ns(r.latency.total_ns),
                r.latency.total_ns.to_string(),
                opt(r.within_ns),
                if r.meets_budget() { "yes" } else { "NO" }.to_string(),
                "generic".to_string(),
            ];
            assert_eq!(row, &expected);
        }
        // The diagram has one lane per link and every term of every link is a bar title.
        assert!(d.html.matches("height=\"16\"").count() >= reports.len());
        for r in &reports {
            assert!(d.html.contains(&format!("{} -&gt; {}", r.from, r.to)));
        }
    }
}

#[test]
fn the_html_is_self_contained() {
    for sources in [lab(), bottling()] {
        let d = make(&sources);
        assert!(d.html.starts_with("<!DOCTYPE html>\n"));
        for banned in [
            "http",
            "<script",
            "src=",
            "href=",
            "url(",
            "@import",
            "<link",
            "<iframe",
            "onload",
            "javascript:",
        ] {
            assert!(!d.html.contains(banned), "html contains `{banned}`");
        }
        assert!(d.html.contains("<svg "));
    }
}

#[test]
fn the_safety_wording_and_the_generic_warning_are_present() {
    for sources in [lab(), bottling()] {
        let d = make(&sources);
        assert_eq!(d.trust, "generic");
        for text in [&d.markdown, &d.html] {
            assert!(text.contains("a design aid, not a certified safety tool"));
            assert!(text.contains("IEC 61508"));
            assert!(text.contains("ISO 13849"));
            assert!(text.contains("A hardwired E-STOP is the authority"));
            assert!(text.contains("generic placeholders"));
            assert!(text.contains("lab session I1"));
        }
        assert!(SAFETY_WORDING.contains("IEC 61508"));
        // The wording is in the summary and again in the safety report.
        assert!(d.markdown.matches("not a certified safety tool").count() >= 2);
    }
}

#[test]
fn the_bottling_dossier_marks_the_devices_that_do_not_build() {
    let d = make(&bottling());
    let state = |dev: &str| d.build_status.iter().find(|s| s.device == dev).unwrap();
    assert_eq!(state("ctl").state, "failed");
    assert_eq!(state("ctl").codes, vec!["NE0610"]);
    assert_eq!(state("arm").state, "failed");
    assert!(state("arm").codes.contains(&"NE0804".to_string()));
    assert_eq!(state("plc").state, "built");
    assert_eq!(state("fpga").state, "built");
    let rows = table_rows(&d.markdown, "## 2.");
    let ctl = rows.iter().find(|r| r[0] == "ctl").unwrap();
    assert_eq!(ctl[2], "failed");
    assert!(ctl[3].contains("NE0610"));
    // What can be computed is still there: links, tags, zones.
    assert!(d.markdown.contains("plc.cap_done"));
    assert!(d.markdown.contains("press"));
    assert!(d.markdown.contains("Zones and mutexes"));
    assert!(d.markdown.contains("Watchdog bounds"));
}

#[test]
fn the_test_report_says_what_ran() {
    let sources = lab();
    let (nir, lib) = analyzed(&sources);
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let d = dossier(&plan, &nir, &sources, &lib, &lab_opts());
    let devices = table_rows(&d.markdown, "## 9.");
    assert!(devices.iter().any(|r| r[3] == "yes"));
    assert!(d.markdown.contains("Simulated time: 300 ms"));
    assert!(d.markdown.contains("Cuts applied: 1"));
    assert!(d
        .markdown
        .contains("Link model: computed worst-case latency"));
    // Link 1 delivered the motor value, within its budget.
    let l1 = d
        .markdown
        .lines()
        .rfind(|l| l.starts_with("| l1 | plc.motor -&gt; arm.go | modbus_tcp |"))
        .unwrap();
    assert!(
        l1.contains("| 23.005 ms | 25 ms | 23.005 ms | 23.005 ms | yes |"),
        "{l1}"
    );
    // The cut of link 0 made the PLC fault, and the report compares it with the bound.
    assert!(d.markdown.contains("### Link loss"));
    assert!(d.markdown.contains("LinkLost"), "{}", d.markdown);
    let none = dossier(
        &partition(&analyzed(&lab()).0, &Library::builtin().unwrap())
            .unwrap()
            .remove(0),
        &analyzed(&lab()).0,
        &lab(),
        &Library::builtin().unwrap(),
        &DossierOptions { sim: None },
    );
    assert!(none.markdown.contains("The co-simulation was not run."));
}
