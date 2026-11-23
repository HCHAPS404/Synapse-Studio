/* neuron-rt port for the Kinetis MKL43Z256VLH4 (FRDM-KL43Z): start-up from flash, GPIO and the
 * 1 ms tick from SysTick. The core runs on the 4 MHz reset clock (MCG-Lite FEI / FIRC); the
 * 48 MHz clock tree is a later improvement. The image runs from flash at 0x00000000 and is
 * flashed by copying app.bin to the OpenSDA mass-storage drive. Register addresses: KL43
 * Sub-Family Reference Manual (KL43P64M48SF6RM, public document).
 *
 * Kinetis GPIO needs three steps, unlike the one-block SAMD21: gate the PORT clock in
 * SIM_SCGC5, set the pin mux (and pull) in the PORT block's PCR, then drive it through the
 * GPIO block (PDDR/PSOR/PCOR/PTOR/PDIR). The PORT and GPIO blocks sit at separate bases, held
 * in lookup tables here.
 *
 * The 16-byte Flash Configuration Field at flash offset 0x400 is MANDATORY on Kinetis: it is
 * emitted here as a const array in the `.flash_config` section (kept by the linker between the
 * vectors and .text) with the standard unsecured value (FSEC = 0xFE, FOPT = 0xFF). Without it
 * the part can be secured on reset and lock.
 *
 * Honest subset: GPIO in/out + input pull-up + the 1 ms tick are the solid core. LPUART0 TX is
 * LEFT OUT (no-op): the LPUART clock-source select cannot be got right without the board. PORT
 * edge interrupts are LEFT OUT: the per-pin IRQC is clear, but the per-port NVIC wiring on KL43
 * (PORTB/PORTE have no dedicated vector on some KL parts) is uncertain without the board, so no
 * interrupt is wired and the build refuses `on interrupt`. ADC and PWM are left out as the other
 * ports do. All of this is noted in the board `note` and the kiro-log.
 */
#include "nrt.h"

#define REG32(a) (*(volatile uint32_t *)(uintptr_t)(a))

/* SIM: clock gating. SIM_SCGC5 gates the PORT clocks (PORTA bit9 .. PORTE bit13). */
#define SIM_SCGC5 REG32(0x40048000u + 0x1038u)

/* PORT blocks (pin mux + pull), one per GPIO port; bases are not evenly derivable here so a
 * table is used. PORT_PCR[pin] = base + 4*pin. */
static const uint32_t KL_PORT_BASE[5] = {
    0x40049000u, /* PORTA */
    0x4004A000u, /* PORTB */
    0x4004B000u, /* PORTC */
    0x4004C000u, /* PORTD */
    0x4004D000u, /* PORTE */
};

/* GPIO blocks (direction + data), one per port. */
static const uint32_t KL_GPIO_BASE[5] = {
    0x400FF000u, /* GPIOA */
    0x400FF040u, /* GPIOB */
    0x400FF080u, /* GPIOC */
    0x400FF0C0u, /* GPIOD */
    0x400FF100u, /* GPIOE */
};

#define PORT_PCR(port, pin) REG32(KL_PORT_BASE[port] + 4u * (uint32_t)(pin))
#define GPIO_PDOR(port) REG32(KL_GPIO_BASE[port] + 0x00u)
#define GPIO_PSOR(port) REG32(KL_GPIO_BASE[port] + 0x04u)
#define GPIO_PCOR(port) REG32(KL_GPIO_BASE[port] + 0x08u)
#define GPIO_PTOR(port) REG32(KL_GPIO_BASE[port] + 0x0Cu)
#define GPIO_PDIR(port) REG32(KL_GPIO_BASE[port] + 0x10u)
#define GPIO_PDDR(port) REG32(KL_GPIO_BASE[port] + 0x14u)

/* PCR bits. MUX field bits 10:8 (001 = GPIO/ALT1), PE (pull enable) bit1, PS (pull select,
 * 1 = up) bit0. */
#define PCR_MUX_GPIO (1u << 8)
#define PCR_PE (1u << 1)
#define PCR_PS (1u << 0)

#define SYST_CSR REG32(0xe000e010u)
#define SYST_RVR REG32(0xe000e014u)
#define SYST_CVR REG32(0xe000e018u)

#ifndef F_CPU
#define F_CPU 4000000u
#endif

static volatile uint32_t g_kl_ms;

void nrt_kl_systick(void);

void nrt_kl_systick(void) { g_kl_ms++; }

void nrt_hal_init(const nrt_app *app) {
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    uint32_t port = d->mask / 32u;
    uint32_t pin = d->mask % 32u;
    if (port > 4u) {
      continue;
    }
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        SIM_SCGC5 |= 1u << (9u + port); /* gate the PORT clock before touching it */
        PORT_PCR(port, pin) = PCR_MUX_GPIO;
        GPIO_PCOR(port) = 1u << pin;
        GPIO_PDDR(port) |= 1u << pin;
        break;
      case NRT_IO_GPIO_IN:
        SIM_SCGC5 |= 1u << (9u + port);
        if (d->cfg & NRT_CFG_PULLUP) {
          PORT_PCR(port, pin) = PCR_MUX_GPIO | PCR_PE | PCR_PS;
        } else {
          PORT_PCR(port, pin) = PCR_MUX_GPIO;
        }
        GPIO_PDDR(port) &= ~(1u << pin);
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

uint32_t nrt_hal_now_ms(void) { return g_kl_ms; }

void nrt_hal_idle(void) { __asm volatile("wfi"); }

void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) {
  return ((GPIO_PDIR(io->mask / 32u) >> (io->mask % 32u)) & 1u) != 0u;
}

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) {
  uint32_t port = io->mask / 32u;
  uint32_t pin = io->mask % 32u;
  if (v) {
    GPIO_PSOR(port) = 1u << pin;
  } else {
    GPIO_PCOR(port) = 1u << pin;
  }
}

void nrt_hal_gpio_flip(const nrt_io_def *io) {
  uint32_t port = io->mask / 32u;
  nrt_crit_t s = nrt_crit_enter();
  GPIO_PTOR(port) = 1u << (io->mask % 32u);
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
  (void)io; /* LPUART0 TX is not implemented in this port (clock-source select deferred to the lab) */
  (void)c;
}

/* ---- flash configuration field ------------------------------------------------------
 * Kinetis parts read 16 bytes at flash offset 0x400 at reset to set flash security. All-zero
 * would SECURE the part and brick it. This is the standard unsecured value: backdoor key and
 * protection all 0xFF, FSEC = 0xFE (unsecured), FOPT = 0xFF. The linker keeps this section at
 * 0x400, between the vectors and .text. Byte values per the spec: 0x400..0x40D = 0xFF,
 * 0x40E = 0xFF (FOPT), 0x40F = 0xFE (FSEC, unsecured). */
__attribute__((section(".flash_config"), used)) const uint8_t nrt_kl_flash_config[16] = {
    0xFF, 0xFF, 0xFF, 0xFF, /* 0x400..0x403 backdoor key */
    0xFF, 0xFF, 0xFF, 0xFF, /* 0x404..0x407 backdoor key */
    0xFF, 0xFF, 0xFF, 0xFF, /* 0x408..0x40B reserved / protection */
    0xFF, 0xFF, 0xFF, 0xFE, /* 0x40C=0xFF 0x40D=0xFF 0x40E=0xFF(FOPT) 0x40F=0xFE(FSEC) */
};

/* ---- start-up ---------------------------------------------------------------------- */

extern uint32_t __data_load__;
extern uint32_t __data_start__;
extern uint32_t __data_end__;
extern uint32_t __bss_start__;
extern uint32_t __bss_end__;
extern uint32_t __stack_top;
int main(void);
void nrt_kl_reset(void);
void nrt_kl_unused(void);

void nrt_kl_reset(void) {
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

void nrt_kl_unused(void) {
  for (;;) {
  }
}

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Woverride-init"
__attribute__((section(".vectors"), used)) void (*const nrt_kl_vectors[16 + 32])(void) = {
    [0] = (void (*)(void))(uintptr_t)&__stack_top,
    [1] = nrt_kl_reset,
    [2 ... 16 + 31] = nrt_kl_unused,
    [15] = nrt_kl_systick,
};
#pragma GCC diagnostic pop
