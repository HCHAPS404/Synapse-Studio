# Design: runtime ports and flash formats (B4, B5)

## Port selection

`targets/runtimes/*.runtime.toml` names a port (`avr`, `rp`, `stm32`, `esp-idf`);
`neuron_codegen_c::Port::for_target(port, chip)` picks the C port and its preprocessor symbol
(`NRT_PORT_AVR`, `NRT_PORT_RP2040`, `NRT_PORT_RP2350`, `NRT_PORT_STM32F4`,
`NRT_PORT_ESP_IDF`). `nrt.h` includes the header of the port.

## Pins in the I/O table

The generated table keeps one form for every port:

- AVR: `NRT_PIN(PORTB, DDRB, PINB, 5)`, register pointers and a bit mask;
- STM32: the same macro, redefined by the port as `port * 16 + bit` (token pasting turns
  `PORTA` into `NRT_PORTA`);
- RP2040, RP2350, ESP32-S3: `NRT_GPIO(n)`, the GPIO number. The hardware check derives `n`
  from chip pins named `GPIOn`.

The AVR output is unchanged, so its goldens hold.

## Bare-metal Arm images

| Port | CPU flags | Linker script | Load address | Clock |
|---|---|---|---|---|
| RP2040 | `-mcpu=cortex-m0plus -mthumb` | `rp_ram.ld` | `0x20000000` (SRAM) | 12 MHz crystal |
| RP2350 | `-mcpu=cortex-m33 -mthumb -mfloat-abi=soft` | `rp_ram.ld` | `0x20000000` | 12 MHz crystal |
| STM32F4 | `-mcpu=cortex-m4 -mthumb -mfloat-abi=soft` | `stm32f446.ld` | `0x08000000` (flash) | 16 MHz HSI |

Start-up is C: the vector table is the first section, the reset handler zeroes `.bss`
(and copies `.data` on STM32), sets VTOR and calls `main`. The RP2350 image carries a minimal
IMAGE_DEF block in its first 4 KiB. `-nostartfiles -lgcc` gives the division and soft-float
helpers.

## UF2

`neuron_codegen_c::uf2`: 512-byte blocks with 256 bytes of payload, the target address,
the block number, the total and the family id (RP2040 `0xe48bff56`, RP2350 Arm secure
`0xe48bff59`).

## Simulation

`neuron-sim-thumb` gained the RP2040 behavior the start-up waits on (RESETS, XOSC, CLOCKS),
the IO_BANK0 edge interrupts on IRQ 13 and the UART0 data register, plus `load_ram` and
`reset_at` to start a RAM image as the boot ROM does.
