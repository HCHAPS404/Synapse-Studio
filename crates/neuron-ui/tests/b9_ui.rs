//! Task B9: the ui dialect on small displays.
//!
//! Property MOC-UI1: the C renderer of the firmware, built for the host, sends bus traffic
//! that the display models turn back into an image equal, pixel for pixel, to the reference
//! renderer of the compiler. The render strategy follows the RAM of the chip, and a display
//! that cannot fit is `NE0604`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_codegen_c::{build_avr, build_host, emit_c_ui, esp_idf_project, find_tool};
use neuron_mir::{lower, Program, Source};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_target::{check, Registry};
use neuron_ui::{emit_ui_c, emu, lower_ui, render, Format, Strategy, UiApp};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

const PANEL: &str = include_str!("../../../examples/panel_ui.neuron");
const U01: &str = include_str!("../../../examples/corpus/ui/u01_status_label.neuron");
const U03: &str = include_str!("../../../examples/corpus/ui/u03_tank_gauge.neuron");
const U04: &str = include_str!("../../../examples/corpus/ui/u04_two_screens.neuron");

fn analysis(files: &[(&str, &str)]) -> neuron_nir::Nir {
    let sources = files
        .iter()
        .map(|(p, t)| SourceFile {
            path: (*p).into(),
            text: t.replace("\r\n", "\n"),
        })
        .collect();
    let a = analyze(sources, None, &SemaOptions::default());
    match a.nir {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    }
}

fn ram(board: &str) -> u64 {
    Registry::builtin()
        .unwrap()
        .resolve(board)
        .unwrap()
        .chip
        .ram_bytes
}

/// The logic (mcu module) and the screen (ui module) of a package.
fn package(mcu: &str, ui: &str) -> (Program, UiApp) {
    let nir = analysis(&[("logic.neuron", mcu), ("screen.neuron", ui)]);
    let hw = check(&Registry::builtin().unwrap(), &nir);
    assert!(hw.diags.is_empty(), "{:?}", hw.diags);
    let l = lower(
        &nir,
        &hw,
        &[Source {
            file: "logic.neuron".into(),
            text: mcu.replace("\r\n", "\n"),
        }],
    );
    assert!(l.diags.is_empty(), "{:?}", l.diags);
    let app = lower_ui(&nir, ram("arduino_uno_r3"), Some(&l.program))
        .unwrap()
        .unwrap();
    (l.program, app)
}

fn have_cc() -> bool {
    if find_tool("NEURON_CC", &["gcc", "cc", "clang"]).is_some() {
        return true;
    }
    assert!(
        std::env::var_os("NEURON_REQUIRE_TOOLCHAIN").is_none(),
        "a host C compiler is required"
    );
    eprintln!("skipped: no host C compiler");
    false
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("neuron-b9-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

const LOGIC: &str = "module logic for mcu on arduino_uno_r3\nio led: gpio.output @ D13\nsignal count: u16\nsignal temp: f32 degC\nsignal hot: bool\ntask tick every 100 ms (prio 1):\n  count = count + 1\n  temp = 21.5 degC\n  hot = count > 2\nsafety:\n  safe_state:\n    led.write(false)\n";

/// Runs the host build for `ms` and returns its trace.
fn host_trace(p: &Program, app: &UiApp, name: &str, ms: u32) -> String {
    let e = emit_c_ui(p, &emit_ui_c(app), app.period_ms);
    let exe = build_host(&e, &scratch(name)).unwrap_or_else(|err| panic!("{err}"));
    let out = Command::new(exe)
        .args(["--for", &ms.to_string()])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The stored values the firmware drew last: `count` from its trace, the others as written.
fn values(trace: &str) -> BTreeMap<String, f32> {
    let mut v = BTreeMap::new();
    for line in trace.lines() {
        if let Some((_, rest)) = line.split_once("SIG    count = ") {
            v.insert("count".to_string(), rest.trim().parse::<f32>().unwrap());
        }
    }
    let count = v.get("count").copied().unwrap_or(0.0);
    v.insert("temp".into(), "294.65".parse::<f32>().unwrap());
    // `hot = count > 2` reads the snapshot taken before the increment: one less than `count`.
    v.insert("hot".into(), if count - 1.0 > 2.0 { 1.0 } else { 0.0 });
    v
}

fn lit(img: &neuron_ui::Image) -> usize {
    img.px
        .iter()
        .filter(|p| **p != 0 && **p != u16::from(b' '))
        .count()
}

#[test]
fn an_oled_shows_what_the_reference_renderer_draws() {
    if !have_cc() {
        return;
    }
    let ui = "module screen for ui on arduino_uno_r3\ndisplay oled: ssd1306(bus: i2c1, size: (128, 64), rotation: 0 deg)\nsignal count: u16\nsignal temp: f32 degC\nsignal hot: bool\nscreen Main on oled:\n  label title \"Oven\" at (0, 0) font sans_16\n  label value bind temp at (0, 20)\n  gauge level bind count range 0..10 at (0, 32)\n  led alarm bind hot at (110, 0)\n";
    let (p, app) = package(LOGIC, ui);
    assert_eq!(app.strategy, Strategy::Framebuffer);
    let trace = host_trace(&p, &app, "oled", 350);
    let got = emu::replay(&trace, &app);
    let want = render(&app, &values(&trace));
    assert!(lit(&want) > 100, "{}", want.to_text());
    assert_eq!(got.to_text(), want.to_text());
    assert!(want.to_text().lines().any(|l| l.contains('#')));
}

#[test]
fn a_color_tft_drawn_in_bands_matches_too() {
    if !have_cc() {
        return;
    }
    // 160x128 at 16 bits is 40 KiB: on the UNO (2 KiB) even a band of 8 rows (2560 bytes) does
    // not fit, so this test uses a smaller panel and the RAM of a larger chip for bands.
    let ui = "module screen for ui on arduino_uno_r3\ndisplay lcd: st7789(bus: spi1, size: (96, 48), rotation: 0 deg)\nsignal count: u16\nscreen Main on lcd:\n  label title \"N\" at (2, 2)\n  bar fill bind count range 0..8 at (60, 2) size (20, 44)\n  slider s bind count range 0..8 at (2, 30) size (50, 12)\n  button b \"OK\" at (2, 12)\n";
    let nir = analysis(&[("logic.neuron", LOGIC), ("screen.neuron", ui)]);
    let hw = check(&Registry::builtin().unwrap(), &nir);
    let l = lower(
        &nir,
        &hw,
        &[Source {
            file: "logic.neuron".into(),
            text: LOGIC.into(),
        }],
    );
    // 96x48x2 = 9216 bytes: bands with 8 KiB of RAM (a band is 1536 bytes).
    let app = lower_ui(&nir, 8192, Some(&l.program)).unwrap().unwrap();
    assert_eq!(app.strategy, Strategy::Bands);
    assert_eq!(app.display.format, Format::Rgb565);
    let trace = host_trace(&l.program, &app, "tft", 350);
    let got = emu::replay(&trace, &app);
    let want = render(&app, &values(&trace));
    assert!(lit(&want) > 50);
    assert_eq!(got.px, want.px);
}

#[test]
fn a_character_lcd_shows_the_text() {
    if !have_cc() {
        return;
    }
    let ui = "module screen for ui on arduino_uno_r3\ndisplay lcd: hd44780(bus: i2c1, size: (16, 2), rotation: 0 deg)\nsignal temp: f32 degC\nsignal hot: bool\nscreen Main on lcd:\n  label t \"Temp\" at (0, 0)\n  label v bind temp at (6, 0)\n  led h bind hot at (15, 1)\n";
    let (p, app) = package(LOGIC, ui);
    assert_eq!(app.strategy, Strategy::CharGrid);
    let trace = host_trace(&p, &app, "lcd", 350);
    let got = emu::replay(&trace, &app);
    let want = render(&app, &values(&trace));
    assert_eq!(got.to_text(), want.to_text());
    // At 300 ms the screen shows count 3, so `hot` is still false.
    assert_eq!(want.to_text(), "TEMP  21.5      \n               O\n");
}

#[test]
fn the_strategy_follows_the_ram_and_ne0604_explains_a_shortfall() {
    // The status panel of the examples: an ILI9341 on the ESP32-S3 has a full framebuffer.
    let nir = analysis(&[("panel.neuron", PANEL)]);
    let app = lower_ui(&nir, ram("esp32_s3_devkit"), None)
        .unwrap()
        .unwrap();
    assert_eq!(app.strategy, Strategy::Framebuffer);
    assert_eq!(app.buffer_bytes, 320 * 240 * 2);
    // The same panel on the UNO: neither a framebuffer nor a band of 8 rows fits in 2 KiB.
    let err = lower_ui(&nir, ram("arduino_uno_r3"), None).unwrap_err();
    assert_eq!(err[0].code, "NE0604");
    assert!(err[0].message.contains("2048"), "{}", err[0].message);
    // The OLED of the corpus fits the UNO with a framebuffer of 1 KiB.
    let app = lower_ui(
        &analysis(&[("u01.neuron", U01)]),
        ram("arduino_uno_r3"),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (app.strategy, app.buffer_bytes),
        (Strategy::Framebuffer, 1024)
    );
    // The tank gauge (ST7789 240x240) on the Pico: 115200 bytes, half of 264 KiB.
    let app = lower_ui(
        &analysis(&[("u03.neuron", U03)]),
        ram("raspberry_pi_pico"),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(app.strategy, Strategy::Framebuffer);
}

#[test]
fn every_screen_of_the_corpus_lowers() {
    for (name, src) in [("u01", U01), ("u03", U03), ("u04", U04), ("panel", PANEL)] {
        let app = lower_ui(&analysis(&[(name, src)]), ram("esp32_s3_devkit"), None)
            .unwrap_or_else(|d| panic!("{name}: {d:?}"))
            .unwrap();
        assert!(!app.widgets.is_empty(), "{name}");
        // A screen renders without any signal written.
        let img = render(&app, &BTreeMap::new());
        assert!(lit(&img) > 0, "{name}");
    }
}

#[test]
fn widgets_outside_the_set_are_ne0610() {
    let src = "module s for ui on esp32_s3_devkit\ndisplay lcd: ili9341(bus: spi1, size: (320, 240), rotation: 0 deg)\nsignal v: f32\nscreen Main on lcd:\n  chart c bind v at (0, 0)\n";
    let a = analyze(
        vec![SourceFile {
            path: "s.neuron".into(),
            text: src.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    if let Some(nir) = a.nir {
        let err = lower_ui(&nir, ram("esp32_s3_devkit"), None).unwrap_err();
        assert!(err.iter().all(|d| d.code == "NE0610"), "{err:?}");
    }
}

#[test]
fn the_uno_firmware_with_an_oled_builds_without_warnings() {
    if find_tool("NEURON_AVR_GCC", &["avr-gcc"]).is_none() {
        assert!(
            std::env::var_os("NEURON_REQUIRE_TOOLCHAIN").is_none(),
            "avr-gcc is required"
        );
        return;
    }
    let ui = "module screen for ui on arduino_uno_r3\ndisplay oled: ssd1306(bus: i2c1, size: (128, 64), rotation: 0 deg)\nsignal count: u16\nscreen Main on oled:\n  label n bind count at (0, 0) font sans_16\n";
    let (p, app) = package(LOGIC, ui);
    let e = emit_c_ui(&p, &emit_ui_c(&app), app.period_ms);
    let b = build_avr(&e, "atmega328p", p.sysclk_hz, &scratch("avr"))
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(b.hex.is_file());
}

#[test]
fn the_esp_idf_project_carries_the_ui_runtime() {
    let nir = analysis(&[("panel.neuron", PANEL)]);
    let app = lower_ui(&nir, ram("esp32_s3_devkit"), None)
        .unwrap()
        .unwrap();
    let p = Program {
        name: "panel".into(),
        board: Some("esp32_s3_devkit".into()),
        sysclk_hz: 240_000_000,
        ..Program::default()
    };
    let files = esp_idf_project(&emit_c_ui(&p, &emit_ui_c(&app), app.period_ms), "panel");
    let cmake = &files
        .iter()
        .find(|(n, _)| n == "main/CMakeLists.txt")
        .unwrap()
        .1;
    assert!(
        cmake.contains("nrt_ui.c") && cmake.contains("nrt_bus_esp_idf.c"),
        "{cmake}"
    );
    assert!(files.iter().any(|(n, _)| n == "main/nrt_ui.h"));
    let app_c = &files.iter().find(|(n, _)| n == "main/app.c").unwrap().1;
    assert!(app_c.contains("NRT_UI_ILI9341") && app_c.contains("static uint8_t ui_buf[153600];"));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: None, ..ProptestConfig::default() })]

    /// A higher value never lights fewer pixels of a bar.
    #[test]
    fn bars_grow_with_their_value(a in 0.0f32..100.0, b in 0.0f32..100.0) {
        let src = "module s for ui on esp32_s3_devkit\ndisplay lcd: st7789(bus: spi1, size: (64, 120), rotation: 0 deg)\nsignal v: f32\nscreen Main on lcd:\n  bar b bind v range 0..100 at (10, 4) size (20, 110)\n";
        let app = lower_ui(&analysis(&[("s.neuron", src)]), ram("esp32_s3_devkit"), None).unwrap().unwrap();
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let count = |v: f32| {
            let mut m = BTreeMap::new();
            m.insert("v".to_string(), v);
            render(&app, &m).px.iter().filter(|p| **p == 0x07e0).count()
        };
        prop_assert!(count(lo) <= count(hi));
    }
}
