//! Task B5 (and B6 for the Thumb simulator): the runtime ports for more boards.
//!
//! The RP2040 firmware that arm-none-eabi-gcc builds from the generated C runs in
//! `neuron-sim-thumb` and drives its pins at the same milliseconds as the interpreter. The
//! STM32F4 and RP2350 images are built (they are not simulated). The ESP-IDF project is
//! generated. The Arm tests skip when `arm-none-eabi-gcc` is missing, unless
//! `NEURON_REQUIRE_ARM` is set (CI sets it).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_codegen_c::{build_arm, emit_c, esp_idf_project, find_tool, uf2, Port};
use neuron_interp::{parse_scenario, render, Interp, Options};
use neuron_mir::{lower, verify, IoK, Program, Source};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_sim_thumb::{Config, Thumb};
use neuron_target::{check, Registry};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

const M01: &str = include_str!("../../../examples/corpus/mcu/m01_blink.neuron");
const M02: &str = include_str!("../../../examples/corpus/mcu/m02_button_toggle.neuron");
const M06: &str = include_str!("../../../examples/corpus/mcu/m06_traffic_light.neuron");
const M08: &str = include_str!("../../../examples/corpus/mcu/m08_two_tasks_signal.neuron");

/// The UNO programs of the corpus, moved to the Pico: D2..D4 become GP2..GP4 and the LED of
/// D13 becomes the board LED on GP25.
fn to_pico(src: &str) -> String {
    src.replace("arduino_uno_r3", "raspberry_pi_pico")
        .replace("@ D13", "@ GP25")
        .replace("@ D2", "@ GP2")
        .replace("@ D3", "@ GP3")
        .replace("@ D4", "@ GP4")
}

fn nir_of(src: &str) -> neuron_nir::Nir {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.replace("\r\n", "\n"),
        }],
        None,
        &SemaOptions::default(),
    );
    match a.nir {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    }
}

fn program(src: &str) -> Program {
    let src = src.replace("\r\n", "\n");
    let nir = nir_of(&src);
    let hw = check(&Registry::builtin().unwrap(), &nir);
    assert!(hw.diags.is_empty(), "{:?}", hw.diags);
    let l = lower(
        &nir,
        &hw,
        &[Source {
            file: "t.neuron".into(),
            text: src,
        }],
    );
    assert!(l.diags.is_empty(), "{:?}", l.diags);
    assert!(verify(&l.program).is_empty());
    l.program
}

fn have_arm() -> bool {
    if find_tool("NEURON_ARM_GCC", &["arm-none-eabi-gcc"]).is_some() {
        return true;
    }
    assert!(
        std::env::var_os("NEURON_REQUIRE_ARM").is_none(),
        "arm-none-eabi-gcc is required (NEURON_REQUIRE_ARM is set) but was not found"
    );
    eprintln!("skipped: arm-none-eabi-gcc not found (see docs/phase-b-prerequisites.md)");
    false
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("neuron-b5-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Builds for the Pico and starts the image in the simulator as the boot ROM starts a RAM UF2.
fn pico(src: &str, name: &str) -> (Program, Thumb) {
    let p = program(&to_pico(src));
    let b = build_arm(&emit_c(&p), Port::Rp2040, p.sysclk_hz, &scratch(name))
        .unwrap_or_else(|e| panic!("{e}"));
    let cfg = Config {
        hz: p.sysclk_hz,
        ..Config::rp2040()
    };
    let mut cpu = Thumb::new(cfg);
    cpu.load_ram(0, &std::fs::read(&b.bin).unwrap()).unwrap();
    cpu.reset_at(cfg.ram_base).unwrap();
    (p, cpu)
}

fn sim_pins(cpu: &Thumb, p: &Program, ms: u32) -> Vec<(u32, String, bool)> {
    let names: BTreeMap<u8, String> = p
        .ios
        .iter()
        .filter(|io| io.kind == IoK::GpioOut)
        .map(|io| (io.bit.unwrap(), io.name.clone()))
        .collect();
    cpu.pin_events()
        .iter()
        .filter_map(|e| {
            names
                .get(&e.pin)
                .map(|n| (cpu.ms(e.cycle).floor() as u32, n.clone(), e.level))
        })
        .filter(|(t, _, _)| *t < ms)
        .collect()
}

fn interp_pins(src: &str, scenario: &str, ms: u32) -> Vec<(u32, String, bool)> {
    let opts = Options {
        end_ns: i64::from(ms) * 1_000_000,
        ..Options::default()
    };
    let mut it = Interp::new(nir_of(src), opts).unwrap();
    it.load_scenario(&parse_scenario(scenario).unwrap());
    it.run();
    let mut last: BTreeMap<String, bool> = BTreeMap::new();
    let mut out = Vec::new();
    for line in render(it.trace()).lines() {
        let Some((time, rest)) = line.split_once(" ms  OUT") else {
            continue;
        };
        let Some((name, value)) = rest.trim().split_once(" = ") else {
            continue;
        };
        if name.contains('.') || !matches!(value, "true" | "false") {
            continue;
        }
        let level = value == "true";
        let prev = last.insert(name.to_string(), level).unwrap_or(false);
        let t = time.trim().parse::<f64>().unwrap().floor() as u32;
        if prev != level && t < ms {
            out.push((t, name.to_string(), level));
        }
    }
    out
}

fn run_ms(cpu: &mut Thumb, ms: u32) {
    let c = cpu.cycle_of_ms(f64::from(ms));
    cpu.run_until(c).unwrap();
}

#[test]
fn blink_on_the_pico_in_the_simulator() {
    if !have_arm() {
        return;
    }
    let (p, mut cpu) = pico(M01, "m01");
    run_ms(&mut cpu, 2100);
    let got = sim_pins(&cpu, &p, 2100);
    assert_eq!(got, interp_pins(&to_pico(M01), "", 2100));
    assert!(got.len() >= 4, "{got:?}");
}

#[test]
fn a_button_interrupt_on_the_pico() {
    if !have_arm() {
        return;
    }
    let (p, mut cpu) = pico(M02, "m02");
    for (ms, level) in [(10.0, true), (20.0, false), (30.0, true), (40.0, false)] {
        let c = cpu.cycle_of_ms(ms);
        cpu.schedule_input(c, 2, level);
    }
    run_ms(&mut cpu, 100);
    let scn = "at 10 ms: button = true\nat 20 ms: button = false\nat 30 ms: button = true\nat 40 ms: button = false\n";
    assert_eq!(
        sim_pins(&cpu, &p, 100),
        interp_pins(&to_pico(M02), scn, 100)
    );
}

#[test]
fn a_state_machine_and_a_signal_on_the_pico() {
    if !have_arm() {
        return;
    }
    let (p, mut cpu) = pico(M06, "m06");
    run_ms(&mut cpu, 9500);
    assert_eq!(
        sim_pins(&cpu, &p, 9500),
        interp_pins(&to_pico(M06), "", 9500)
    );
    let (p, mut cpu) = pico(M08, "m08");
    run_ms(&mut cpu, 200);
    assert_eq!(sim_pins(&cpu, &p, 200), interp_pins(&to_pico(M08), "", 200));
}

#[test]
fn a_uart_print_on_the_pico() {
    if !have_arm() {
        return;
    }
    let src = "module hello for mcu on raspberry_pi_pico\nio dbg: uart(baud: 115200 Bd) @ UART0\nvar n: u16 = 0\ntask say every 100 ms (prio 1):\n  n += 1\n  dbg.print(\"n={n}\\n\")\nsafety:\n  safe_state:\n    pass\n";
    let (_, mut cpu) = pico(src, "uart");
    run_ms(&mut cpu, 250);
    let text: String = cpu.uart_output().iter().map(|(_, b)| *b as char).collect();
    assert_eq!(text, "n=1\nn=2\nn=3\n");
}

#[test]
fn the_pico_image_and_its_uf2_file() {
    if !have_arm() {
        return;
    }
    let p = program(&to_pico(M01));
    let b = build_arm(&emit_c(&p), Port::Rp2040, p.sysclk_hz, &scratch("uf2")).unwrap();
    let image = std::fs::read(&b.bin).unwrap();
    let f = std::fs::read(b.uf2.unwrap()).unwrap();
    assert_eq!(f.len(), image.len().div_ceil(256) * 512);
    // The vector table is first: the stack pointer at the top of SRAM.
    assert_eq!(
        u32::from_le_bytes([image[0], image[1], image[2], image[3]]),
        0x2004_2000
    );
    assert_eq!(b.load_address, 0x2000_0000);
}

#[test]
fn the_nucleo_and_the_pico_2_w_images_build() {
    if !have_arm() {
        return;
    }
    // LD2, the green LED of the board, is PA5.
    let nucleo = M01
        .replace("arduino_uno_r3", "nucleo_f446re")
        .replace("@ D13", "@ PA5");
    let p = program(&nucleo);
    let b = build_arm(&emit_c(&p), Port::Stm32f4, p.sysclk_hz, &scratch("f446")).unwrap();
    assert!(b.uf2.is_none());
    let image = std::fs::read(&b.bin).unwrap();
    // The reset vector points into flash, with the Thumb bit set.
    let reset = u32::from_le_bytes([image[4], image[5], image[6], image[7]]);
    assert!(
        (0x0800_0000..0x0808_0000).contains(&reset) && reset & 1 == 1,
        "{reset:#x}"
    );

    let w = M02
        .replace("arduino_uno_r3", "raspberry_pi_pico_2w")
        .replace("@ D13", "@ GP15")
        .replace("@ D2", "@ GP2");
    let p = program(&w);
    let b = build_arm(&emit_c(&p), Port::Rp2350, p.sysclk_hz, &scratch("rp2350")).unwrap();
    let image = std::fs::read(&b.bin).unwrap();
    // The image definition block of the RP2350 boot ROM is in the first 4 KiB.
    let words: Vec<u32> = image[..4096.min(image.len())]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let at = words
        .iter()
        .position(|w| *w == 0xffff_ded3)
        .expect("block start");
    assert_eq!(
        &words[at..at + 5],
        &[0xffff_ded3, 0x1021_0142, 0x0000_01ff, 0, 0xab12_3579]
    );
}

#[test]
fn the_esp_idf_project_has_its_files() {
    let src = M01
        .replace("arduino_uno_r3", "esp32_s3_devkit")
        .replace("@ D13", "@ GPIO2");
    let p = program(&src);
    let files = esp_idf_project(&emit_c(&p), "m01_blink");
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    for want in [
        "CMakeLists.txt",
        "main/CMakeLists.txt",
        "main/app.c",
        "main/nrt.h",
        "main/nrt_core.c",
        "main/nrt_port_esp_idf.c",
    ] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    let app = &files.iter().find(|(n, _)| n == "main/app.c").unwrap().1;
    assert!(app.contains("NRT_GPIO(2)"), "{app}");
    assert!(files[0].1.contains("project(m01_blink)"));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    /// The UF2 writer covers the image exactly: block count, consecutive addresses, payload.
    #[test]
    fn uf2_blocks_cover_the_image(len in 0usize..2000, seed in any::<u8>()) {
        let data: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect();
        let f = uf2::uf2(&data, 0x2000_0000, uf2::FAMILY_RP2040);
        let blocks = len.div_ceil(256).max(1);
        prop_assert_eq!(f.len(), blocks * 512);
        let mut back = Vec::new();
        for (i, b) in f.chunks(512).enumerate() {
            let addr = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
            prop_assert_eq!(addr, 0x2000_0000 + 256 * i as u32);
            back.extend_from_slice(&b[32..32 + 256]);
        }
        prop_assert_eq!(&back[..len], &data[..]);
    }
}
