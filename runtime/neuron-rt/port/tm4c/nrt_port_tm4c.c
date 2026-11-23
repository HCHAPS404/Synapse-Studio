/* neuron-rt port for the TI TM4C123GH6PM (EK-TM4C123GXL, Tiva C LaunchPad): start-up from
 * flash at 0x00000000, GPIO, input pull-up and the 1 ms tick from SysTick. The core runs on
 * the 16 MHz precision internal oscillator (PIOSC), which is the reset clock, so the port
 * needs no clock set-up beyond enabling each port's clock. The PLL (up to 80 MHz) is a later
 * improvement. Register addresses: public TM4C123GH6PM datasheet (document SPMS376).
 *
 * TM4C GPIO is NOT the STM32 layout: each port's clock must be enabled in SYSCTL RCGCGPIO
 * (then wait on PRGPIO) before the port is touched, the port base addresses are not a linear
 * stride (a static base table is used), and the data register is written address-masked.
 *
 * Honest subset: GPIO in/out + input pull-up + the 1 ms SysTick tick are the solid core and
 * are what the blink example needs. UART0 TX and per-port edge interrupts are LEFT OUT until
 * the board session (TM4C GPIO interrupts are per-port, not per-line like STM32 EXTI, so they
 * are risky to wire blind): uart_put is a no-op and no interrupt vectors are installed, the
 * same way the STM32 ports leave out ADC and PWM. ADC and PWM are also left out (return 0 /
 * no-op); the build refuses programs that use them.
 */
#include "nrt.h"

#define REG(a) (*(volatile uint32_t *)(uintptr_t)(a))

/* System control: enable a port's clock, then wait until it is ready. */
#define SYSCTL_RCGCGPIO REG(0x400FE000u + 0x608u)
#define SYSCTL_PRGPIO REG(0x400FE000u + 0xA08u)

/* GPIO APB port bases, indexed by port number (A=0..F=5). NOT a linear stride: A..D are
 * 0x1000 apart from 0x40004000, E and F are at 0x40024000/0x40025000. */
static const uint32_t g_tm4c_gpio_base[6] = {
    0x40004000u, /* A */
    0x40005000u, /* B */
    0x40006000u, /* C */
    0x40007000u, /* D */
    0x40024000u, /* E */
    0x40025000u, /* F */
};

/* GPIO register offsets (within a port base). */
#define GPIO_DIR 0x400u
#define GPIO_DEN 0x51Cu
#define GPIO_PUR 0x510u
#define GPIO_DATA_ALL 0x3FCu /* read/write every pin (all-pins address mask) */

#define SYST_CSR REG(0xE000E010u)
#define SYST_RVR REG(0xE000E014u)
#define SYST_CVR REG(0xE000E018u)

#ifndef F_CPU
#define F_CPU 16000000u
#endif

static volatile uint32_t g_tm4c_ms;

void nrt_tm4c_systick(void);
void nrt_tm4c_systick(void) { g_tm4c_ms++; }

/* Enable a port's clock and wait for it (datasheet requires the port be ready first). */
static void tm4c_port_clock(uint32_t port) {
  SYSCTL_RCGCGPIO |= 1u << port;
  while ((SYSCTL_PRGPIO & (1u << port)) == 0u) {
  }
}

void nrt_hal_init(const nrt_app *app) {
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    uint32_t port = d->mask / 16u;
    uint32_t bit = d->mask % 16u;
    if (port > 5u) {
      continue;
    }
    uint32_t base = g_tm4c_gpio_base[port];
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        tm4c_port_clock(port);
        REG(base + GPIO_DIR) |= 1u << bit;
        REG(base + GPIO_DEN) |= 1u << bit;
        /* drive low initially */
        REG(base + (1u << (bit + 2u))) = 0x00u;
        break;
      case NRT_IO_GPIO_IN:
        tm4c_port_clock(port);
        REG(base + GPIO_DIR) &= ~(1u << bit);
        REG(base + GPIO_DEN) |= 1u << bit;
        if (d->cfg & NRT_CFG_PULLUP) {
          REG(base + GPIO_PUR) |= 1u << bit;
        }
        break;
      default:
        /* UART, ADC and PWM are left out of this port; see the file header. */
        break;
    }
  }
}

void nrt_hal_start(void) {
  SYST_RVR = (uint32_t)(F_CPU / 1000u) - 1u;
  SYST_CVR = 0u;
  SYST_CSR = 7u; /* ENABLE | TICKINT | CLKSOURCE (core clock) */
  __asm volatile("cpsie i" : : : "memory");
}

uint32_t nrt_hal_now_ms(void) { return g_tm4c_ms; }

void nrt_hal_idle(void) { __asm volatile("wfi"); }

void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) {
  uint32_t port = io->mask / 16u;
  uint32_t bit = io->mask % 16u;
  if (port > 5u) {
    return false;
  }
  return ((REG(g_tm4c_gpio_base[port] + GPIO_DATA_ALL) >> bit) & 1u) != 0u;
}

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) {
  uint32_t port = io->mask / 16u;
  uint32_t bit = io->mask % 16u;
  if (port > 5u) {
    return;
  }
  /* Address-masked single-pin write: the bit's address selects only that pin. */
  REG(g_tm4c_gpio_base[port] + (1u << (bit + 2u))) = v ? 0xFFu : 0x00u;
}

void nrt_hal_gpio_flip(const nrt_io_def *io) {
  uint32_t port = io->mask / 16u;
  uint32_t bit = io->mask % 16u;
  if (port > 5u) {
    return;
  }
  uint32_t base = g_tm4c_gpio_base[port];
  nrt_crit_t s = nrt_crit_enter();
  uint32_t cur = REG(base + GPIO_DATA_ALL);
  REG(base + (1u << (bit + 2u))) = ((cur >> bit) & 1u) ? 0x00u : 0xFFu;
  nrt_crit_leave(s);
}

uint16_t nrt_hal_adc_get(const nrt_io_def *io) {
  (void)io; /* not in this port: the build refuses programs that read an ADC */
  return 0u;
}

void nrt_hal_pwm_put(const nrt_io_def *io, float duty) {
  (void)io; /* not in this port: the build refuses programs with PWM */
  (void)duty;
}

void nrt_hal_uart_put(const nrt_io_def *io, char c) {
  (void)io; /* UART0 TX is left out of this port until the board session (see the header) */
  (void)c;
}

/* ---- start-up ---------------------------------------------------------------------- */

extern uint32_t __data_load__;
extern uint32_t __data_start__;
extern uint32_t __data_end__;
extern uint32_t __bss_start__;
extern uint32_t __bss_end__;
extern uint32_t __stack_top;
int main(void);
void nrt_tm4c_reset(void);
void nrt_tm4c_unused(void);

void nrt_tm4c_reset(void) {
  const uint32_t *src = &__data_load__;
  for (uint32_t *p = &__data_start__; p < &__data_end__; p++) {
    *p = *src++;
  }
  for (uint32_t *p = &__bss_start__; p < &__bss_end__; p++) {
    *p = 0u;
  }
  (void)main();
  for (;;) {
  }
}

void nrt_tm4c_unused(void) {
  for (;;) {
  }
}

/* The TM4C123 vector table: 16 system vectors + 139 interrupt vectors. Only the stack top,
 * reset and SysTick (system vector 15) are populated; the rest trap. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Woverride-init"
__attribute__((section(".vectors"), used)) void (*const nrt_tm4c_vectors[16 + 139])(void) = {
    [0] = (void (*)(void))(uintptr_t)&__stack_top,
    [1] = nrt_tm4c_reset,
    [2 ... 16 + 138] = nrt_tm4c_unused,
    [15] = nrt_tm4c_systick,
};
#pragma GCC diagnostic pop
