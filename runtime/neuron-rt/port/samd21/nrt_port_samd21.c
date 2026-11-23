/* neuron-rt port for the SAMD21G18A (Arduino Nano 33 IoT): start-up from flash, GPIO and the
 * 1 ms tick from SysTick. The core runs on the 8 MHz internal oscillator (OSC8M, the reset
 * oscillator's source before the /8 prescaler is cleared); the Arduino core's 48 MHz DFLL is a
 * later improvement. The application is linked above the SAM-BA bootloader (0x00002000) and
 * flashed with bossac. Register addresses: SAM D21 family datasheet (public document).
 *
 * Honest subset: GPIO in/out + input pull-up + the 1 ms tick are the solid core. UART TX and
 * EIC edge interrupts are LEFT OUT in this version: SERCOM USART and the EIC both need GCLK
 * routing that cannot be got right without the board. `nrt_hal_uart_put` is a no-op and no
 * external interrupt is wired, so a program that only uses gpio and a tick runs; one that needs
 * UART output or an edge interrupt will build but those features do nothing (noted in the board
 * `note` and the kiro-log). ADC and PWM are left out as the STM32F4 port does.
 */
#include "nrt.h"

#define REG8(a) (*(volatile uint8_t *)(uintptr_t)(a))
#define REG32(a) (*(volatile uint32_t *)(uintptr_t)(a))

/* PORT base 0x41004400; group stride 0x80 (group 0 = PA, group 1 = PB). */
#define PORT_BASE(g) (0x41004400u + 0x80u * (g))
#define PORT_DIRSET(g) REG32(PORT_BASE(g) + 0x08u)
#define PORT_DIRCLR(g) REG32(PORT_BASE(g) + 0x04u)
#define PORT_OUTSET(g) REG32(PORT_BASE(g) + 0x18u)
#define PORT_OUTCLR(g) REG32(PORT_BASE(g) + 0x14u)
#define PORT_OUTTGL(g) REG32(PORT_BASE(g) + 0x1cu)
#define PORT_IN(g) REG32(PORT_BASE(g) + 0x20u)
#define PORT_PINCFG(g, n) REG8(PORT_BASE(g) + 0x40u + (n))

/* PINCFG bits: bit0 PMUXEN, bit1 INEN, bit2 PULLEN. OUTSET/OUTCLR picks the pull direction. */
#define PINCFG_INEN 0x02u
#define PINCFG_PULLEN 0x04u

#define SYST_CSR REG32(0xe000e010u)
#define SYST_RVR REG32(0xe000e014u)
#define SYST_CVR REG32(0xe000e018u)

#ifndef F_CPU
#define F_CPU 8000000u
#endif

static volatile uint32_t g_sd_ms;

void nrt_sd_systick(void);

void nrt_sd_systick(void) { g_sd_ms++; }

void nrt_hal_init(const nrt_app *app) {
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    uint32_t grp = d->mask / 32u;
    uint32_t pin = d->mask % 32u;
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        PORT_OUTCLR(grp) = 1u << pin;
        PORT_PINCFG(grp, pin) = 0u;
        PORT_DIRSET(grp) = 1u << pin;
        break;
      case NRT_IO_GPIO_IN:
        PORT_DIRCLR(grp) = 1u << pin;
        if (d->cfg & NRT_CFG_PULLUP) {
          PORT_PINCFG(grp, pin) = (uint8_t)(PINCFG_INEN | PINCFG_PULLEN);
          PORT_OUTSET(grp) = 1u << pin; /* OUT=1 selects pull-up */
        } else {
          PORT_PINCFG(grp, pin) = (uint8_t)PINCFG_INEN;
        }
        break;
      default:
        /* UART, ADC and PWM are not implemented in this port. */
        break;
    }
  }
}

void nrt_hal_start(void) {
  SYST_RVR = (uint32_t)(F_CPU / 1000u) - 1u;
  SYST_CVR = 0u;
  SYST_CSR = 7u;
  __asm volatile("cpsie i" : : : "memory");
}

uint32_t nrt_hal_now_ms(void) { return g_sd_ms; }

void nrt_hal_idle(void) { __asm volatile("wfi"); }

void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) {
  return ((PORT_IN(io->mask / 32u) >> (io->mask % 32u)) & 1u) != 0u;
}

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) {
  uint32_t grp = io->mask / 32u;
  uint32_t pin = io->mask % 32u;
  if (v) {
    PORT_OUTSET(grp) = 1u << pin;
  } else {
    PORT_OUTCLR(grp) = 1u << pin;
  }
}

void nrt_hal_gpio_flip(const nrt_io_def *io) {
  uint32_t grp = io->mask / 32u;
  nrt_crit_t s = nrt_crit_enter();
  PORT_OUTTGL(grp) = 1u << (io->mask % 32u);
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
  (void)io; /* UART TX is not implemented in this port (SERCOM+GCLK deferred to the lab session) */
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
void nrt_sd_reset(void);
void nrt_sd_unused(void);

void nrt_sd_reset(void) {
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

void nrt_sd_unused(void) {
  for (;;) {
  }
}

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Woverride-init"
__attribute__((section(".vectors"), used)) void (*const nrt_sd_vectors[16 + 28])(void) = {
    [0] = (void (*)(void))(uintptr_t)&__stack_top,
    [1] = nrt_sd_reset,
    [2 ... 16 + 27] = nrt_sd_unused,
    [15] = nrt_sd_systick,
};
#pragma GCC diagnostic pop
