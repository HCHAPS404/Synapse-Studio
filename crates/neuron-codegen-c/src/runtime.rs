//! The runtime sources, embedded so that `neuron build` works from any directory.

/// A runtime file to write next to the generated C.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFile {
    /// File name.
    pub name: &'static str,
    /// Text.
    pub text: &'static str,
}

/// A port of the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Port {
    /// AVR microcontrollers (ATmega328P, ATmega2560).
    Avr,
    /// The host, with simulated time (differential tests).
    Host,
    /// Raspberry Pi RP2040 (Cortex-M0+), bare metal, image in SRAM.
    Rp2040,
    /// Raspberry Pi RP2350 (Cortex-M33), bare metal, image in SRAM.
    Rp2350,
    /// STM32F4 (Cortex-M4), bare metal, image in flash.
    Stm32f4,
    /// ESP32-S3 on ESP-IDF.
    EspIdf,
}

impl Port {
    /// The port for a runtime file of `targets/runtimes` and a chip, if there is one.
    pub fn for_target(runtime_port: &str, chip: &str) -> Option<Port> {
        match (runtime_port, chip) {
            ("avr", _) => Some(Port::Avr),
            ("rp", "rp2040") => Some(Port::Rp2040),
            ("rp", "rp2350") => Some(Port::Rp2350),
            ("stm32", c) if c.starts_with("stm32f4") => Some(Port::Stm32f4),
            ("esp-idf", _) => Some(Port::EspIdf),
            _ => None,
        }
    }

    /// The preprocessor symbol that selects the port.
    pub fn define(self) -> &'static str {
        match self {
            Port::Avr => "NRT_PORT_AVR",
            Port::Host => "NRT_HOST",
            Port::Rp2040 => "NRT_PORT_RP2040",
            Port::Rp2350 => "NRT_PORT_RP2350",
            Port::Stm32f4 => "NRT_PORT_STM32F4",
            Port::EspIdf => "NRT_PORT_ESP_IDF",
        }
    }
}

const NRT_H: &str = include_str!("../../../runtime/neuron-rt/include/nrt.h");
const NRT_CORE: &str = include_str!("../../../runtime/neuron-rt/src/nrt_core.c");
const AVR_H: &str = include_str!("../../../runtime/neuron-rt/port/avr/nrt_port_avr.h");
const AVR_C: &str = include_str!("../../../runtime/neuron-rt/port/avr/nrt_port_avr.c");
const HOST_H: &str = include_str!("../../../runtime/neuron-rt/port/host/nrt_port_host.h");
const HOST_C: &str = include_str!("../../../runtime/neuron-rt/port/host/nrt_port_host.c");
const RP_H: &str = include_str!("../../../runtime/neuron-rt/port/rp/nrt_port_rp.h");
const RP_C: &str = include_str!("../../../runtime/neuron-rt/port/rp/nrt_port_rp.c");
const RP_LD: &str = include_str!("../../../runtime/neuron-rt/port/rp/rp_ram.ld");
const ST_H: &str = include_str!("../../../runtime/neuron-rt/port/stm32f4/nrt_port_stm32f4.h");
const ST_C: &str = include_str!("../../../runtime/neuron-rt/port/stm32f4/nrt_port_stm32f4.c");
const ST_LD: &str = include_str!("../../../runtime/neuron-rt/port/stm32f4/stm32f446.ld");
const ESP_H: &str = include_str!("../../../runtime/neuron-rt/port/esp_idf/nrt_port_esp_idf.h");
const ESP_C: &str = include_str!("../../../runtime/neuron-rt/port/esp_idf/nrt_port_esp_idf.c");
const UI_H: &str = include_str!("../../../runtime/neuron-rt/ui/nrt_ui.h");
const UI_C: &str = include_str!("../../../runtime/neuron-rt/ui/nrt_ui.c");
const BUS_HOST: &str = include_str!("../../../runtime/neuron-rt/port/host/nrt_bus_host.c");
const BUS_AVR: &str = include_str!("../../../runtime/neuron-rt/port/avr/nrt_bus_avr.c");
const BUS_ESP: &str = include_str!("../../../runtime/neuron-rt/port/esp_idf/nrt_bus_esp_idf.c");

/// The ui renderer and the display bus of a port, or `None` when the port has no bus driver
/// yet (the bare-metal Arm ports).
pub fn ui_files(port: Port) -> Option<Vec<RuntimeFile>> {
    let f = |name, text| RuntimeFile { name, text };
    let bus = match port {
        Port::Host => f("nrt_bus_host.c", BUS_HOST),
        Port::Avr => f("nrt_bus_avr.c", BUS_AVR),
        Port::EspIdf => f("nrt_bus_esp_idf.c", BUS_ESP),
        Port::Rp2040 | Port::Rp2350 | Port::Stm32f4 => return None,
    };
    Some(vec![f("nrt_ui.h", UI_H), f("nrt_ui.c", UI_C), bus])
}

/// The files of a port, headers first. Linker scripts end in `.ld`.
pub fn runtime_files(port: Port) -> Vec<RuntimeFile> {
    let f = |name, text| RuntimeFile { name, text };
    let mut v = vec![f("nrt.h", NRT_H), f("nrt_core.c", NRT_CORE)];
    match port {
        Port::Avr => v.extend([f("nrt_port_avr.h", AVR_H), f("nrt_port_avr.c", AVR_C)]),
        Port::Host => v.extend([f("nrt_port_host.h", HOST_H), f("nrt_port_host.c", HOST_C)]),
        Port::Rp2040 | Port::Rp2350 => v.extend([
            f("nrt_port_rp.h", RP_H),
            f("nrt_port_rp.c", RP_C),
            f("rp_ram.ld", RP_LD),
        ]),
        Port::Stm32f4 => v.extend([
            f("nrt_port_stm32f4.h", ST_H),
            f("nrt_port_stm32f4.c", ST_C),
            f("stm32f446.ld", ST_LD),
        ]),
        Port::EspIdf => v.extend([
            f("nrt_port_esp_idf.h", ESP_H),
            f("nrt_port_esp_idf.c", ESP_C),
        ]),
    }
    v
}
