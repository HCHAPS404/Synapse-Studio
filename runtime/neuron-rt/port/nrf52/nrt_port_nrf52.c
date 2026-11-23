/* neuron-rt port for the nRF52840 (Arduino Nano 33 BLE Sense Rev2): start-up from flash, GPIO,
 * GPIOTE edge interrupts, the 1 ms tick from SysTick and UARTE0 transmit (EasyDMA, one byte at a
 * time). The core runs on the 64 MHz high-frequency internal oscillator (HFINT, the reset
 * clock); the application is linked above the Adafruit UF2 bootloader (0x00026000). Register
 * addresses: nRF52840 product specification (public document).
 *
 * Honest subset (like the STM32F4 port): ADC and PWM are left out (the build refuses programs
 * that use them). GPIO in/out + input pull-up + the 1 ms tick are the solid core. UARTE0 TX and
 * GPIOTE edge interrupts are best-effort and not yet run on the board.
 */
#include "nrt.h"

#define REG(a) (*(volatile uint32_t *)(uintptr_t)(a))

/* GPIO: P0 base 0x50000000, P1 base 0x50000300. */
#define GPIO_BASE(p) (0x50000000u + 0x300u * (p))
#define GPIO_OUTSET(p) REG(GPIO_BASE(p) + 0x508u)
#define GPIO_OUTCLR(p) REG(GPIO_BASE(p) + 0x50cu)
#define GPIO_IN(p) REG(GPIO_BASE(p) + 0x510u)
#define GPIO_DIRSET(p) REG(GPIO_BASE(p) + 0x518u)
#define GPIO_DIRCLR(p) REG(GPIO_BASE(p) + 0x51cu)
#define GPIO_PIN_CNF(p, n) REG(GPIO_BASE(p) + 0x700u + 4u * (n))

/* GPIOTE base 0x40006000. */
#define GPIOTE_TASKS_OUT(i) REG(0x40006000u + 4u * (i))
#define GPIOTE_EVENTS_IN(i) REG(0x40006100u + 4u * (i))
#define GPIOTE_INTENSET REG(0x40006304u)
#define GPIOTE_CONFIG(i) REG(0x40006510u + 4u * (i))
#define GPIOTE_IRQ 6u

/* UARTE0 base 0x40002000. */
#define UARTE_TASKS_STARTTX REG(0x40002008u)
#define UARTE_TASKS_STOPTX REG(0x4000200cu)
#define UARTE_EVENTS_ENDTX REG(0x40002120u)
#define UARTE_ENABLE REG(0x40002500u)
#define UARTE_PSEL_TXD REG(0x4000250cu)
#define UARTE_BAUDRATE REG(0x40002524u)
#define UARTE_TXD_PTR REG(0x40002544u)
#define UARTE_TXD_MAXCNT REG(0x40002548u)
#define UARTE_BAUD_115200 0x01d7e000u

#define SYST_CSR REG(0xe000e010u)
#define SYST_RVR REG(0xe000e014u)
#define SYST_CVR REG(0xe000e018u)
#define NVIC_ISER(i) REG(0xe000e100u + 4u * (i))

#ifndef F_CPU
#define F_CPU 64000000u
#endif

static const nrt_app *g_nrf_app;
static volatile uint32_t g_nrf_ms;
static volatile uint8_t g_nrf_tx; /* one-byte EasyDMA buffer */

void nrt_nrf_systick(void);
void nrt_nrf_gpiote(void);

void nrt_nrf_systick(void) { g_nrf_ms++; }

/* One handler for every GPIOTE channel: the event registers say which fired. A channel is
 * allocated per input pin that asks for an edge, in I/O-table order. */
void nrt_nrf_gpiote(void) {
  if (g_nrf_app == NULL) {
    return;
  }
  uint8_t ch = 0u;
  for (uint8_t i = 0; i < g_nrf_app->n_io; i++) {
    const nrt_io_def *d = &g_nrf_app->io[i];
    if (d->kind != NRT_IO_GPIO_IN || d->aux == NRT_NO_IRQ) {
      continue;
    }
    uint32_t edge = (d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u;
    if (edge == 0u || ch >= 8u) {
      continue;
    }
    if (GPIOTE_EVENTS_IN(ch) != 0u) {
      GPIOTE_EVENTS_IN(ch) = 0u;
      nrt_dispatch_irq(d->aux);
    }
    ch++;
  }
}

void nrt_hal_init(const nrt_app *app) {
  g_nrf_app = app;
  uint8_t ch = 0u;
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    uint32_t port = d->mask / 32u;
    uint32_t pin = d->mask % 32u;
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        GPIO_OUTCLR(port) = 1u << pin;
        /* PIN_CNF: DIR=1 (output), INPUT disconnected, no pull, standard drive. */
        GPIO_PIN_CNF(port, pin) = 1u | (1u << 1);
        GPIO_DIRSET(port) = 1u << pin;
        break;
      case NRT_IO_GPIO_IN: {
        /* PIN_CNF: DIR=0 (input), INPUT connected (bit1=0); PULL=3 (up) when asked. */
        uint32_t cnf = 0u;
        if (d->cfg & NRT_CFG_PULLUP) {
          cnf |= 3u << 2;
        }
        GPIO_DIRCLR(port) = 1u << pin;
        GPIO_PIN_CNF(port, pin) = cnf;
        uint32_t edge = (d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u;
        if (edge != 0u && d->aux != NRT_NO_IRQ && ch < 8u) {
          /* GPIOTE CONFIG: mode=1 (event), PSEL=pin, PORT bit13=port, polarity bits16..17. */
          uint32_t polarity = (edge == 3u) ? 3u : ((edge & 2u) ? 1u : 2u);
          GPIOTE_CONFIG(ch) = 1u | (pin << 8) | (port << 13) | (polarity << 16);
          GPIOTE_EVENTS_IN(ch) = 0u;
          GPIOTE_INTENSET = 1u << ch;
          ch++;
        }
        break;
      }
      case NRT_IO_UART:
        /* UARTE0 TX only, 115200 Bd, one-byte EasyDMA buffer on the pin in `tx`. */
        UARTE_PSEL_TXD = (port << 5) | pin; /* PSEL: bits0..4 pin, bit5 port */
        UARTE_BAUDRATE = UARTE_BAUD_115200;
        UARTE_TXD_PTR = (uint32_t)(uintptr_t)&g_nrf_tx;
        UARTE_TXD_MAXCNT = 1u;
        UARTE_ENABLE = 8u; /* 8 = enable UARTE */
        break;
      default:
        break;
    }
  }
  if (ch > 0u) {
    NVIC_ISER(GPIOTE_IRQ / 32u) = 1u << (GPIOTE_IRQ % 32u);
  }
}

void nrt_hal_start(void) {
  SYST_RVR = (uint32_t)(F_CPU / 1000u) - 1u;
  SYST_CVR = 0u;
  SYST_CSR = 7u;
  __asm volatile("cpsie i" : : : "memory");
}

uint32_t nrt_hal_now_ms(void) { return g_nrf_ms; }

void nrt_hal_idle(void) { __asm volatile("wfi"); }

void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) {
  return ((GPIO_IN(io->mask / 32u) >> (io->mask % 32u)) & 1u) != 0u;
}

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) {
  uint32_t port = io->mask / 32u;
  uint32_t pin = io->mask % 32u;
  if (v) {
    GPIO_OUTSET(port) = 1u << pin;
  } else {
    GPIO_OUTCLR(port) = 1u << pin;
  }
}

void nrt_hal_gpio_flip(const nrt_io_def *io) {
  uint32_t port = io->mask / 32u;
  uint32_t pin = io->mask % 32u;
  nrt_crit_t s = nrt_crit_enter();
  if ((GPIO_IN(port) >> pin) & 1u) {
    GPIO_OUTCLR(port) = 1u << pin;
  } else {
    GPIO_OUTSET(port) = 1u << pin;
  }
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
  (void)io;
  g_nrf_tx = (uint8_t)c;
  UARTE_EVENTS_ENDTX = 0u;
  UARTE_TASKS_STARTTX = 1u;
  while (UARTE_EVENTS_ENDTX == 0u) {
  }
  UARTE_TASKS_STOPTX = 1u;
}

/* ---- start-up ---------------------------------------------------------------------- */

extern uint32_t __data_load__;
extern uint32_t __data_start__;
extern uint32_t __data_end__;
extern uint32_t __bss_start__;
extern uint32_t __bss_end__;
extern uint32_t __stack_top;
int main(void);
void nrt_nrf_reset(void);
void nrt_nrf_unused(void);

void nrt_nrf_reset(void) {
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

void nrt_nrf_unused(void) {
  for (;;) {
  }
}

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Woverride-init"
__attribute__((section(".vectors"), used)) void (*const nrt_nrf_vectors[16 + 48])(void) = {
    [0] = (void (*)(void))(uintptr_t)&__stack_top,
    [1] = nrt_nrf_reset,
    [2 ... 16 + 47] = nrt_nrf_unused,
    [15] = nrt_nrf_systick,
    [16 + 6] = nrt_nrf_gpiote,
};
#pragma GCC diagnostic pop
