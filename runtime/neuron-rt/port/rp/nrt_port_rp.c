/* neuron-rt port for the RP2040 and RP2350: start-up, clocks, GPIO through SIO, edge
 * interrupts from IO_BANK0, the 1 ms tick from SysTick and UART0 transmit.
 *
 * Clocks: the crystal oscillator (12 MHz on the Pico boards) feeds clk_ref and clk_sys, so the
 * core runs at 12 MHz. Running the PLL at 125 MHz is a later improvement; the tick does not
 * depend on it. Register addresses: RP2040 and RP2350 datasheets (public documents).
 */
#include "nrt.h"

#define REG(a) (*(volatile uint32_t *)(uintptr_t)(a))

#if defined(NRT_PORT_RP2350)
#define RESETS_BASE 0x40020000u
#define RST_IO_BANK0 (1u << 6)
#define RST_PADS_BANK0 (1u << 9)
#define RST_UART0 (1u << 26)
#define CLOCKS_BASE 0x40010000u
#define XOSC_BASE 0x40048000u
#define IO_BANK0_BASE 0x40028000u
#define IO_INTR0 0x230u
#define IO_INTE0 0x248u
#define IO_INTS0 0x278u
#define PADS_BASE 0x40038000u
#define PAD_ISO (1u << 8)
#define UART0_BASE 0x40070000u
#define IO_IRQ 21u
#define SIO_OUT_SET 0xd0000018u
#define SIO_OUT_CLR 0xd0000020u
#define SIO_OUT_XOR 0xd0000028u
#define SIO_OE_SET 0xd0000038u
#define SIO_OE_CLR 0xd0000040u
#define N_VECTORS 68
#else
#define RESETS_BASE 0x4000c000u
#define RST_IO_BANK0 (1u << 5)
#define RST_PADS_BANK0 (1u << 8)
#define RST_UART0 (1u << 22)
#define CLOCKS_BASE 0x40008000u
#define XOSC_BASE 0x40024000u
#define IO_BANK0_BASE 0x40014000u
#define IO_INTR0 0x0f0u
#define IO_INTE0 0x100u
#define IO_INTS0 0x120u
#define PADS_BASE 0x4001c000u
#define PAD_ISO 0u
#define UART0_BASE 0x40034000u
#define IO_IRQ 13u
#define SIO_OUT_SET 0xd0000014u
#define SIO_OUT_CLR 0xd0000018u
#define SIO_OUT_XOR 0xd000001cu
#define SIO_OE_SET 0xd0000024u
#define SIO_OE_CLR 0xd0000028u
#define N_VECTORS 48
#endif

#define SIO_IN 0xd0000004u
#define RESETS_RESET REG(RESETS_BASE + 0x0u)
#define RESETS_DONE REG(RESETS_BASE + 0x8u)
#define XOSC_CTRL REG(XOSC_BASE + 0x00u)
#define XOSC_STATUS REG(XOSC_BASE + 0x04u)
#define XOSC_STARTUP REG(XOSC_BASE + 0x0cu)
#define CLK_REF_CTRL REG(CLOCKS_BASE + 0x30u)
#define CLK_REF_SELECTED REG(CLOCKS_BASE + 0x38u)
#define CLK_SYS_CTRL REG(CLOCKS_BASE + 0x3cu)
#define CLK_SYS_SELECTED REG(CLOCKS_BASE + 0x44u)
#define CLK_PERI_CTRL REG(CLOCKS_BASE + 0x48u)
#define GPIO_CTRL(n) REG(IO_BANK0_BASE + 8u * (n) + 4u)
#define IO_INTR(i) REG(IO_BANK0_BASE + IO_INTR0 + 4u * (i))
#define IO_INTE(i) REG(IO_BANK0_BASE + IO_INTE0 + 4u * (i))
#define IO_INTS(i) REG(IO_BANK0_BASE + IO_INTS0 + 4u * (i))
#define PAD(n) REG(PADS_BASE + 4u + 4u * (n))
#define UART_DR REG(UART0_BASE + 0x00u)
#define UART_FR REG(UART0_BASE + 0x18u)
#define UART_IBRD REG(UART0_BASE + 0x24u)
#define UART_FBRD REG(UART0_BASE + 0x28u)
#define UART_LCR_H REG(UART0_BASE + 0x2cu)
#define UART_CR REG(UART0_BASE + 0x30u)
#define SYST_CSR REG(0xe000e010u)
#define SYST_RVR REG(0xe000e014u)
#define SYST_CVR REG(0xe000e018u)
#define NVIC_ISER REG(0xe000e100u)
#define SCB_VTOR REG(0xe000ed08u)

#define FUNC_SIO 5u
#define FUNC_UART 2u
#define PAD_DEFAULT 0x56u /* input enable, 4 mA, pull-down, Schmitt trigger */
#define PAD_PULLUP 0x5au  /* input enable, 4 mA, pull-up, Schmitt trigger */

#ifndef F_CPU
#define F_CPU 12000000u
#endif

static const nrt_app *g_rp_app;
static volatile uint32_t g_rp_ms;

void nrt_rp_systick(void);
void nrt_rp_io_irq(void);

void nrt_rp_systick(void) { g_rp_ms++; }

void nrt_rp_io_irq(void) {
  if (g_rp_app == NULL) {
    return;
  }
  for (uint8_t i = 0; i < g_rp_app->n_io; i++) {
    const nrt_io_def *d = &g_rp_app->io[i];
    if (d->kind != NRT_IO_GPIO_IN || d->aux == NRT_NO_IRQ) {
      continue;
    }
    uint32_t n = d->mask;
    uint32_t shift = 4u * (n % 8u);
    uint32_t st = (IO_INTS(n / 8u) >> shift) & 0xcu;
    if (st != 0u) {
      IO_INTR(n / 8u) = st << shift; /* write 1 to clear the edge */
      nrt_dispatch_irq(d->aux);
    }
  }
}

static void rp_clocks(void) {
  XOSC_STARTUP = 47u;                  /* about 1 ms at 12 MHz */
  XOSC_CTRL = (0xfabu << 12) | 0xaa0u; /* enable, 1 to 15 MHz range */
  while ((XOSC_STATUS & 0x80000000u) == 0u) {
  }
  CLK_REF_CTRL = 2u; /* clk_ref from the crystal oscillator */
  while ((CLK_REF_SELECTED & (1u << 2)) == 0u) {
  }
  CLK_SYS_CTRL = 0u; /* clk_sys from clk_ref */
  while ((CLK_SYS_SELECTED & 1u) == 0u) {
  }
  CLK_PERI_CTRL = 1u << 11; /* clk_peri enabled, from clk_sys */
}

void nrt_hal_init(const nrt_app *app) {
  g_rp_app = app;
  uint32_t release = RST_IO_BANK0 | RST_PADS_BANK0 | RST_UART0;
  RESETS_RESET &= ~release;
  while ((RESETS_DONE & release) != release) {
  }
  rp_clocks();
  bool irq = false;
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    uint32_t n = d->mask;
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        PAD(n) = PAD_DEFAULT;
        GPIO_CTRL(n) = FUNC_SIO;
        REG(SIO_OUT_CLR) = 1u << n;
        REG(SIO_OE_SET) = 1u << n;
        break;
      case NRT_IO_GPIO_IN: {
        PAD(n) = (d->cfg & NRT_CFG_PULLUP) ? PAD_PULLUP : PAD_DEFAULT;
        GPIO_CTRL(n) = FUNC_SIO;
        REG(SIO_OE_CLR) = 1u << n;
        uint32_t edge = (d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u; /* 1 falling, 2 rising, 3 both */
        if (edge != 0u && d->aux != NRT_NO_IRQ) {
          uint32_t bits = ((edge & 1u) ? 0x4u : 0u) | ((edge & 2u) ? 0x8u : 0u);
          IO_INTR(n / 8u) = 0xcu << (4u * (n % 8u));
          IO_INTE(n / 8u) |= bits << (4u * (n % 8u));
          irq = true;
        }
        break;
      }
      case NRT_IO_UART:
        /* UART0 transmit on GP0 at 115200 Bd: 12 MHz / (16 * 115200) = 6 + 33/64. */
        PAD(0u) = PAD_DEFAULT & ~0x04u;
        GPIO_CTRL(0u) = FUNC_UART;
        UART_IBRD = 6u;
        UART_FBRD = 33u;
        UART_LCR_H = 0x70u; /* 8 data bits, FIFOs on */
        UART_CR = 0x101u;   /* UART and transmitter on */
        break;
      default:
        break;
    }
#if defined(NRT_PORT_RP2350)
    PAD(n) &= ~PAD_ISO;
#endif
  }
  if (irq) {
    NVIC_ISER = 1u << IO_IRQ;
  }
}

void nrt_hal_start(void) {
  SYST_RVR = (uint32_t)(F_CPU / 1000u) - 1u;
  SYST_CVR = 0u;
  SYST_CSR = 7u; /* core clock, interrupt, enable */
  __asm volatile("cpsie i" : : : "memory");
}

uint32_t nrt_hal_now_ms(void) { return g_rp_ms; }

void nrt_hal_idle(void) { __asm volatile("wfi"); }

void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) { return ((REG(SIO_IN) >> io->mask) & 1u) != 0u; }

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) { REG(v ? SIO_OUT_SET : SIO_OUT_CLR) = 1u << io->mask; }

void nrt_hal_gpio_flip(const nrt_io_def *io) { REG(SIO_OUT_XOR) = 1u << io->mask; }

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
  while ((UART_FR & (1u << 5)) != 0u) { /* transmit FIFO full */
  }
  UART_DR = (uint32_t)(uint8_t)c;
}

/* ---- start-up ---------------------------------------------------------------------- */

extern uint32_t __bss_start__;
extern uint32_t __bss_end__;
extern uint32_t __stack_top;
int main(void);
void nrt_rp_reset(void);
void nrt_rp_unused(void);

void nrt_rp_reset(void) {
  for (uint32_t *p = &__bss_start__; p < &__bss_end__; p++) {
    *p = 0u;
  }
  extern void (*const nrt_rp_vectors[])(void);
  SCB_VTOR = (uint32_t)(uintptr_t)nrt_rp_vectors;
  (void)main();
  for (;;) {
  }
}

void nrt_rp_unused(void) {
  for (;;) {
  }
}

/* The vector table is the first thing of the image (the boot ROM takes SP and PC from it).
 * Every entry defaults to a trap, then the used ones are set (a GNU range initializer). */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Woverride-init"
__attribute__((section(".vectors"), used)) void (*const nrt_rp_vectors[N_VECTORS])(void) = {
    [0] = (void (*)(void))(uintptr_t)&__stack_top,
    [1] = nrt_rp_reset,
    [2 ... N_VECTORS - 1] = nrt_rp_unused,
    [15] = nrt_rp_systick,
    [16 + IO_IRQ] = nrt_rp_io_irq,
};
#pragma GCC diagnostic pop

#if defined(NRT_PORT_RP2350)
/* Minimal image definition for the RP2350 boot ROM: an Arm secure executable for the RP2350
 * (IMAGE_TYPE item), the LAST item, no further block. It must be in the first 4 KiB. */
__attribute__((section(".vectors"), used)) const uint32_t nrt_rp2350_image_def[5] = {
    0xffffded3u, /* block start */
    0x10210142u, /* IMAGE_TYPE: EXE | SECURE | ARM | RP2350 */
    0x000001ffu, /* LAST, size of the items in words */
    0x00000000u, /* link to the next block: none */
    0xab123579u, /* block end */
};
#endif
