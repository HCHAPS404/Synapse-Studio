# 07 · Hardware

This chapter applies to `mcu`, `plc`, `hdl`, `robot` and `ui` modules, and to
devices in a `cell`. The device-specific facts (pins, peripherals, clocks, memory,
voltages) come from the **target description stack**:

| Layer | File | Content |
|---|---|---|
| Architecture | `targets/arch/<name>.arch.toml` | ISA, word size, endianness, FPU, atomic access widths, interrupt controller model, ABI, Harvard/von Neumann |
| Chip | `targets/chips/<name>.chip.toml` (+ CMSIS-SVD) | memory map, flash/RAM, peripherals and registers, clock tree, pin multiplexing, voltage domains |
| Board | `targets/boards/<name>.board.toml` | crystal, on-board LEDs/buttons/connectors, voltages, programmer and bootloader |
| Platform runtime | `targets/runtimes/<name>.toml` | bare metal, vendor SDK (e.g. ESP-IDF) or Linux user space; startup code; scheduler port |

Adding a device never changes the compiler: it adds or reuses these files.

## 7.1 I/O bindings

Every use of a hardware resource goes through an `io` binding:

```neuron
io led:     gpio.output                                   @ PA5
io btn:     gpio.input(pull: up, debounce: 20 ms)          @ PC13
io dbg:     uart(baud: 115200 Bd, parity: none)            @ USART2(tx: PA2, rx: PA3)
io heater:  pwm(freq: 1 kHz)                               @ TIM2.ch1(PA0)
io temp:    adc(resolution: 12, sample: 15 cycles)         @ ADC1.in4(PA4)
io bus:     i2c(speed: 400 kHz)                            @ I2C1(scl: PB8, sda: PB9)
io flash:   spi(mode: 0, clock: 8 MHz)                     @ SPI1(sck: PA5, miso: PA6, mosi: PA7, cs: PB6)
io can0:    can(bitrate: 500 kHz)                          @ CAN1(tx: PB9, rx: PB8)
io tick:    timer(period: 1 ms)                            @ TIM6
```

Form: `io <name>: <kind>(<config>) @ <location>`.

- `<kind>` is a peripheral interface from `std.hal` (`gpio.input`, `gpio.output`,
  `gpio.open_drain`, `adc`, `dac`, `pwm`, `timer`, `capture`, `encoder`, `uart`,
  `spi`, `i2c`, `can`, `usb_cdc`, `i2s`, `rtc`, `watchdog`, ...).
- `<location>` names the chip resource and its pins, resolved in the target.
- The compiler checks that the pin supports that function (alternate-function
  table), that the configuration is achievable from the clock tree, and that no
  resource is bound twice.

> **Property HW-1 (exclusive ownership).** Every hardware resource (pin, peripheral
> instance, channel, DMA stream) is owned by at most one `io` binding or one
> `uses regs` declaration. Violations are `NE0601`.

## 7.2 Peripheral operations

Each kind exposes typed operations, for example:

```neuron
led.write(true); led.toggle(); let pressed = not btn.read()
dbg.print("count={presses}")          # non-blocking, buffered; returns false when full
heater.duty = 35 %
let v = temp.read() |> scale(0..4095 -> 0..3.3 V)
```

Operations that would block are not offered; asynchronous operations return a handle
(`busy`, `done`, `failed`) usable from `fsm` and `sequence`, or deliver events
(`on interrupt dbg.rx`).

## 7.3 Clock tree

```neuron
clocks:
  sysclk = 168 MHz from hse(8 MHz)
  apb1   = sysclk / 4
  apb2   = sysclk / 2
```

- The compiler's **clock solver** finds PLL multipliers and dividers that produce the
  requested frequencies within the chip's limits, or fails with `NE0605` listing the
  nearest achievable values.
- All derived rates (baud rates, PWM frequencies, timer periods, SPI and I2C clocks,
  ADC sampling) are computed from the tree. The achieved error is reported; above
  the tolerance (`@tolerance(2 %)` by default for UART) it is `NE0606`.
- Without a `clocks:` block, the board default (reset clock) is used.

## 7.4 Typed registers

For hardware not covered by `std.hal`, modules can access registers generated from
the chip's SVD:

```neuron
uses regs.GPIOB, regs.TIM3

fn fast_toggle():
  regs.GPIOB.BSRR.BS7 = 1           # write-only field, typed
  let busy = regs.TIM3.SR.UIF       # read field
  regs.TIM3.SR.UIF = clear          # write-1-to-clear field
```

- Field types and access rights (`read-only`, `write-only`, `read-write`,
  `write-1-to-clear`) come from the SVD; violating them is `NE0602`.
- Enumerated field values from the SVD are named constants.
- `uses regs.X` takes ownership of peripheral `X` (HW-1).

## 7.5 Memory-mapped and placed data

```neuron
@section(".noinit")
var crash_log: [u8; 256]

@place(ccm)            # a memory region named in the chip file
var fast_buffer: [i16; 1024]
```

## 7.6 Inline assembly and foreign code

```neuron
trusted asm thumb2(out r: u32 = r0, clobber: [r1]):
  mrs r0, PRIMASK

trusted extern "C":
  fn vendor_lib_init(config: u32) -> i32
```

- `asm` blocks are per architecture; a module MAY provide several with `@when`.
- `asm` and `extern` code is **unverified**: it SHALL be marked `trusted`, it is
  excluded from proofs, and it is listed in the build's audit report (08).
- `pub extern "C" fn` exports a Neuron function with the C ABI.

## 7.7 Architecture-aware compilation

The compiler adapts code generation to the architecture without changing the
program's meaning:

| Concern | Behavior |
|---|---|
| Word size and atomicity | Shared variables wider than the atomic access width get critical sections (05) |
| Harvard architectures | Read-only data is placed in program memory with the right access instructions |
| No FPU | Software float with cost reports; fixed-point suggestions (03) |
| Interrupt controller | Priorities mapped to the controller (NVIC, AVR vectors, Xtensa levels, PLIC/CLIC) |
| Multicore | Tasks assigned to cores; cross-core state only through channels and signals |
| Platform runtime | Bare-metal startup, or scheduler ported onto the vendor SDK (e.g. FreeRTOS inside ESP-IDF) |
| Memory budget | MEM-1 (05) |

## 7.8 Electrical model

Board and device manifests declare the voltage domain and electrical limits of every
pin (logic levels, maximum current, 5 V tolerance). Links between devices in a
`cell` and wiring in the digital twin are checked:
3.3 V outputs to 24 V inputs (or the reverse) require an isolation or level-shifting
element in the link (`via ... opto_24v`, `level_shifter`) or fail with `NE0801`.

## 7.9 Hardware simulation

Every `io` kind has a simulation model so that programs run unchanged on virtual
devices in the digital twin; the instruction-set simulators execute the same binary
that is flashed to the real chip.
