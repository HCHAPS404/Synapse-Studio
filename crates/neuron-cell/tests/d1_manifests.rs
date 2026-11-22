//! Task D1.1: manifest schema, loader, trust levels and `NE0607`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{duration_ns, rate_bd, Kind, Library, Manifest, ManifestError, Medium, Trust};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::Path;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const HOST: &str = r#"
[device]
name = "demo_host"
kind = "host"
dialects = ["host", "core"]
backend = "python"
trust = "generic"
note = "placeholder numbers"
[io]
gpio = { prefix = "P", volts = 3.3 }
[timing]
scan_min = "1 ms"
modbus_tcp = { transport = "2 ms", jitter = "500 us" }
[safety]
safe_state_time = "10 ms"
"#;

fn nir_of(text: &str) -> neuron_nir::Nir {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    a.nir.clone().unwrap_or_else(|| panic!("{}", a.render()))
}

#[test]
fn builtin_library_loads_robots_and_media() {
    let lib = Library::builtin().unwrap();
    let scara = lib.device("generic_scara").unwrap();
    assert_eq!(scara.kind, Kind::Robot);
    assert_eq!(scara.trust, Trust::Generic);
    assert!(scara.robot.is_some());
    let z = lib.medium("zero_hop").unwrap();
    assert_eq!(z.isolating, ["opto_24v"]);
}

#[test]
fn trust_levels_are_ordered() {
    assert!(Trust::Generic < Trust::Measured && Trust::Measured < Trust::Confirmed);
}

#[test]
fn units_use_the_analyzer_table() {
    assert_eq!(duration_ns("500 us"), Ok(500_000));
    assert_eq!(duration_ns("1.5 ms"), Ok(1_500_000));
    assert_eq!(rate_bd("100 MBd"), Ok(100_000_000));
    assert!(duration_ns("3 V").is_err());
    assert!(duration_ns("3 parsecs").is_err());
    assert!(duration_ns("ms").is_err());
    assert!(duration_ns("10").is_err());
}

#[test]
fn invalid_files_are_ne0607() {
    let cases = [
        ("not toml [", "cannot read"),
        (&HOST.replace("python", "scl"), "cannot build"),
        (
            &HOST.replace("\"host\", \"core\"", "\"cobol\""),
            "unknown dialect",
        ),
        (
            &HOST.replace("500 us", "500 volts"),
            "timing.modbus_tcp.jitter",
        ),
        (
            &HOST.replace("note = \"placeholder numbers\"", ""),
            "`note` is required",
        ),
        (
            &HOST.replace("[io]", "[io]\nx = { prefix = \"P\", volts = -1.0 }"),
            "out of range",
        ),
        (
            &HOST
                .replace("kind = \"host\"", "kind = \"mcu\"")
                .replace("python", "c"),
            "device.target",
        ),
        (
            &HOST.replace("trust = \"generic\"", "trust = \"guessed\""),
            "cannot read",
        ),
        (
            &HOST.replace("[safety]", "[safety]\nextra = 1"),
            "cannot read",
        ),
    ];
    for (text, needle) in cases {
        let e = Manifest::parse("demo_host.toml", text).unwrap_err();
        assert_eq!(e.code(), "NE0607");
        assert!(e.to_string().contains(needle), "{e} lacks `{needle}`");
    }
    assert!(Manifest::parse("demo_host.toml", HOST).is_ok());
}

#[test]
fn medium_checks_isolating_options() {
    let text = "[medium]\nname=\"w\"\nfamily=\"zero_hop\"\ntransport_worst=\"1 us\"\nspeaks=[\"mcu\"]\ntrust=\"measured\"\noptions=[\"a\"]\nisolating=[\"b\"]\n";
    let e = Medium::parse("w.toml", text).unwrap_err();
    assert!(e.to_string().contains("isolating `b`"));
}

#[test]
fn library_rejects_wrong_folder_name_and_dangling_medium() {
    let mut lib = Library::builtin().unwrap();
    let e = lib.add_text("plcs/demo_host.toml", HOST).unwrap_err();
    assert!(e.to_string().contains("does not belong"));
    let e = lib.add_text("hosts/other.toml", HOST).unwrap_err();
    assert!(e.to_string().contains("differs from the file name"));
    lib.add_text(
        "hosts/demo_host.toml",
        &HOST.replace("modbus_tcp", "carrier_pigeon"),
    )
    .unwrap();
    let e = lib.check().unwrap_err();
    assert!(e.to_string().contains("unknown medium `carrier_pigeon`"));
    let mut ok = Library::builtin().unwrap();
    ok.add_text("hosts/demo_host.toml", HOST).unwrap();
    ok.check().unwrap();
    assert!(ok.device("demo_host").is_some());
}

#[test]
fn directory_loading_matches_the_embedded_files() {
    let from_disk = Library::from_dir(&root().join("devices")).unwrap();
    let built = Library::builtin().unwrap();
    assert_eq!(from_disk.device_names(), built.device_names());
    assert_eq!(from_disk.medium_names(), built.medium_names());
    for n in built.device_names() {
        assert_eq!(from_disk.device(&n), built.device(&n));
    }
}

#[test]
fn unknown_device_and_medium_in_a_cell_are_ne0607() {
    let lib = Library::builtin().unwrap();
    let nir = nir_of(
        "cell C\n  device a: generic_scara @ usb\n  device b: no_such_device @ usb\n  link a.x -> b.y via carrier_pigeon within 5 ms\n  link a.x -> b.z via zero_hop(gpio, magic)\n",
    );
    let errs = lib.resolve_cell(&nir).unwrap_err();
    assert_eq!(errs.len(), 3, "{errs:?}");
    assert!(errs.iter().all(|e| e.code() == "NE0607"));
    assert!(matches!(
        &errs[0],
        ManifestError::Unknown { kind: "device", .. }
    ));
    assert!(errs[1].to_string().contains("carrier_pigeon"));
    assert!(errs[2].to_string().contains("no option `magic`"));
}

proptest! {
    #[test]
    fn durations_are_exact(n in 0u64..1_000_000, u in 0usize..4) {
        let (unit, scale) = [("ns", 1u64), ("us", 1_000), ("ms", 1_000_000), ("s", 1_000_000_000)][u];
        prop_assert_eq!(duration_ns(&format!("{n} {unit}")), Ok(n * scale));
    }

    #[test]
    fn parsing_never_panics_and_errors_carry_the_code(text in ".{0,200}") {
        if let Err(e) = Manifest::parse("x.toml", &text) {
            prop_assert_eq!(e.code(), "NE0607");
        }
        if let Err(e) = Medium::parse("x.toml", &text) {
            prop_assert_eq!(e.code(), "NE0607");
        }
        let _ = duration_ns(&text);
    }

    #[test]
    fn manifest_values_round_trip(scan in 1u64..10_000, tr in 0u64..10_000, ss in 1u64..1_000) {
        let text = HOST
            .replace("1 ms", &format!("{scan} us"))
            .replace("2 ms", &format!("{tr} us"))
            .replace("10 ms", &format!("{ss} ms"));
        let m = Manifest::parse("demo_host.toml", &text).unwrap();
        prop_assert_eq!(m.scan_min_ns, Some(scan * 1_000));
        prop_assert_eq!(m.timing["modbus_tcp"].transport_ns, tr * 1_000);
        prop_assert_eq!(m.safe_state_ns, ss * 1_000_000);
    }
}
