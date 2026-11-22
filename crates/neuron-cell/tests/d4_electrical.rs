//! Task D4.2: electrical compatibility of wired links (`NE0801`).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{check_electrical, link_table, partition, CellDiag, Library};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::Path;

fn check(text: &str) -> Vec<CellDiag> {
    let a = analyze(
        vec![SourceFile {
            path: "c.neuron".into(),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let rows = link_table(&nir, &plan, &lib).unwrap();
    let man = |i: &str| plan.device(i).map(|d| d.manifest.clone());
    check_electrical(&rows, &plan.file, &man, &lib)
}

const FPGA_PLC: &str =
    "cell C\n  device fpga: generic_artix7_board @ usb\n  device plc: generic_s7_1200 @ usb\n";

#[test]
fn a_3v3_output_on_a_24v_input_without_opto_is_ne0801() {
    let d = check(&format!(
        "{FPGA_PLC}  link fpga.x -> plc.y via zero_hop(gpio) within 5 ms\n"
    ));
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0801");
    assert!(d[0].message.contains("3.3 V output") && d[0].message.contains("24 V input"));
    assert!(
        d[0].help[0].contains("zero_hop(gpio, opto_24v)"),
        "{:?}",
        d[0].help
    );
    assert!(d[0].span.is_some());
}

#[test]
fn the_isolating_option_makes_it_legal() {
    let d = check(&format!(
        "{FPGA_PLC}  link fpga.x -> plc.y via zero_hop(gpio, opto_24v) within 5 ms\n"
    ));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn same_domain_wires_and_network_media_are_fine() {
    let two = "cell C\n  device a: nucleo_f446re @ usb\n  device b: generic_artix7_board @ usb\n  link a.x -> b.y via zero_hop(gpio) within 1 ms\n";
    assert!(check(two).is_empty());
    let net = "cell C\n  device a: nucleo_f446re @ usb\n  device p: generic_s7_1200 @ usb\n  link a.x -> p.y via modbus_tcp every 20 ms within 50 ms\n";
    assert!(check(net).is_empty());
}

#[test]
fn a_3v3_board_driving_a_5v_board_is_ne0801() {
    let d = check("cell C\n  device a: nucleo_f446re @ usb\n  device u: arduino_uno_r3 @ usb\n  link a.x -> u.y via zero_hop(gpio) within 5 ms\n");
    assert_eq!(d.len(), 1);
    assert!(d[0].message.contains("5 V input"), "{}", d[0]);
}

#[test]
fn the_corpus_and_bottling_links_are_electrically_sound() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut texts: Vec<String> = ["x01_two_devices", "x02_robot_zone", "x04_event_from_fpga"]
        .iter()
        .map(|n| std::fs::read_to_string(root.join(format!("corpus/cell/{n}.neuron"))).unwrap())
        .collect();
    // The bottling cell's links, without the modules it places.
    let bottling = std::fs::read_to_string(root.join("bottling_line.neuron")).unwrap();
    texts.push(
        bottling
            .lines()
            .filter(|l| !l.trim_start().starts_with("place "))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    for t in texts {
        assert!(check(&t).is_empty(), "{t}");
    }
}
