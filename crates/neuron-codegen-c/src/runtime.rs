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
}

const NRT_H: &str = include_str!("../../../runtime/neuron-rt/include/nrt.h");
const NRT_CORE: &str = include_str!("../../../runtime/neuron-rt/src/nrt_core.c");
const AVR_H: &str = include_str!("../../../runtime/neuron-rt/port/avr/nrt_port_avr.h");
const AVR_C: &str = include_str!("../../../runtime/neuron-rt/port/avr/nrt_port_avr.c");
const HOST_H: &str = include_str!("../../../runtime/neuron-rt/port/host/nrt_port_host.h");
const HOST_C: &str = include_str!("../../../runtime/neuron-rt/port/host/nrt_port_host.c");

/// The files of a port, headers first.
pub fn runtime_files(port: Port) -> Vec<RuntimeFile> {
    let core = [
        RuntimeFile {
            name: "nrt.h",
            text: NRT_H,
        },
        RuntimeFile {
            name: "nrt_core.c",
            text: NRT_CORE,
        },
    ];
    let specific = match port {
        Port::Avr => [
            RuntimeFile {
                name: "nrt_port_avr.h",
                text: AVR_H,
            },
            RuntimeFile {
                name: "nrt_port_avr.c",
                text: AVR_C,
            },
        ],
        Port::Host => [
            RuntimeFile {
                name: "nrt_port_host.h",
                text: HOST_H,
            },
            RuntimeFile {
                name: "nrt_port_host.c",
                text: HOST_C,
            },
        ],
    };
    core.into_iter().chain(specific).collect()
}
