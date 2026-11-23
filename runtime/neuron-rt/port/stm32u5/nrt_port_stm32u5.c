/* neuron-rt port for the STM32U585 (Arduino UNO Q, the STM32 side): start-up from flash, GPIO,
 * EXTI edge interrupts, the 1 ms tick from SysTick and USART1 transmit. The core runs on the
 * 16 MHz internal oscillator (HSI16); the reset clock is MSIS at 4 MHz, so the port turns HSI on
 * and selects it. The PLL is a later improvement. Register addresses: STM32U585 reference manual
 * (public document). GPIO is the modern STM32 layout (same registers as F4/L4/U5); the AHB2 base
 * and the RCC enable register differ from F4 and come from the U5 manual.
 *
 * Honest subset: GPIO in/out + input pull-up + the 1 ms tick are the solid core; EXTI edge
 * interrupts and USART1 TX are best-effort, not yet run on the board. ADC and PWM are left out
 * as the STM32F4 port does. The on-board user LED pin of the UNO Q is not confirmed for this
 * variant; the board file uses PA5 as a placeholder (to confirm on the board).
 */
#include "nrt.h"

#define REG(a) (*(volatile uint32_t *)(uintptr_t)(a))

/* RCC base 0x46020800 (U5). */
#define RCC_CR REG(0x46020800u + 0x00u)
#define RCC_CFGR1 REG(0x46020800u + 0x1cu)
#define RCC_AHB2ENR1 REG(0x46020800u + 0x8cu)
#define RCC_APB2ENR REG(0x46020800u + 0xa0u)

/* GPIO base 0x42020000 (AHB2), port stride 0x400. */
#define GPIO_BASE(p) (0x42020000u + 0x400u * (p))
#define GPIO_MODER(p) REG(GPIO_BASE(p) + 0x00u)
#define GPIO_PUPDR(p) REG(GPIO_BASE(p) + 0x0cu)
#define GPIO_IDR(p) REG(GPIO_BASE(p) + 0x10u)
#define GPIO_ODR(p) REG(GPIO_BASE(p) + 0x14u)
#define GPIO_BSRR(p) REG(GPIO_BASE(p) + 0x18u)
#define GPIO_AFRL(p) REG(GPIO_BASE(p) + 0x20u)

/* EXTI base 0x46022000 (U5). EXTICR replaces the F4 SYSCFG_EXTICR. */
#define EXTI_RTSR1 REG(0x46022000u + 0x00u)
#define EXTI_FTSR1 REG(0x46022000u + 0x04u)
#define EXTI_RPR1 REG(0x46022000u + 0x0cu)
#define EXTI_FPR1 REG(0x46022000u + 0x10u)
#define EXTI_EXTICR(i) REG(0x46022000u + 0x60u + 4u * (i))
#define EXTI_IMR1 REG(0x46022000u + 0x80u)

/* USART1 base 0x40013800 (U5), oversampling-16 BRR like F4. */
#define USART1_CR1 REG(0x40013800u + 0x00u)
#define USART1_BRR REG(0x40013800u + 0x0cu)
#define USART1_ISR REG(0x40013800u + 0x1cu)
#define USART1_TDR REG(0x40013800u + 0x28u)

#define SYST_CSR REG(0xe000e010u)
#define SYST_RVR REG(0xe000e014u)
#define SYST_CVR REG(0xe000e018u)
#define NVIC_ISER(i) REG(0xe000e100u + 4u * (i))

#ifndef F_CPU
#define F_CPU 16000000u
#endif

static const nrt_app *g_u5_app;
static volatile uint32_t g_u5_ms;

void nrt_u5_systick(void);
void nrt_u5_exti(void);

void nrt_u5_systick(void) { g_u5_ms++; }

/* One handler for every EXTI line: the pending bits say which lines fired. */
void nrt_u5_exti(void) {
  uint32_t rp = EXTI_RPR1;
  uint32_t fp = EXTI_FPR1;
  EXTI_RPR1 = rp;
  EXTI_FPR1 = fp;
  uint32_t pr = rp | fp;
  if (g_u5_app == NULL) {
    return;
  }
  for (uint8_t i = 0; i < g_u5_app->n_io; i++) {
    const nrt_io_def *d = &g_u5_app->io[i];
    if (d->kind == NRT_IO_GPIO_IN && d->aux != NRT_NO_IRQ && (pr & (1u << (d->mask % 16u))) != 0u) {
      nrt_dispatch_irq(d->aux);
    }
  }
}

/* NVIC number of the EXTI line on U5 (EXTI0..15 are IRQ 11..26). */
static uint32_t exti_irq(uint32_t line) { return 11u + line; }

void nrt_hal_init(const nrt_app *app) {
  g_u5_app = app;
  /* Turn HSI16 on and select it as the system clock (reset clock is MSIS 4 MHz). */
  RCC_CR |= 1u << 8;              /* HSION */
  while ((RCC_CR & (1u << 10)) == 0u) { /* wait HSIRDY */
  }
  RCC_CFGR1 = (RCC_CFGR1 & ~3u) | 1u; /* SW = 01: HSI16 */
  RCC_AHB2ENR1 |= 0xffu;          /* GPIOA..GPIOH clocks */
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    uint32_t port = d->mask / 16u;
    uint32_t pin = d->mask % 16u;
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        GPIO_BSRR(port) = 1u << (pin + 16u);
        GPIO_MODER(port) = (GPIO_MODER(port) & ~(3u << (2u * pin))) | (1u << (2u * pin));
        break;
      case NRT_IO_GPIO_IN: {
        GPIO_MODER(port) &= ~(3u << (2u * pin));
        if (d->cfg & NRT_CFG_PULLUP) {
          GPIO_PUPDR(port) = (GPIO_PUPDR(port) & ~(3u << (2u * pin))) | (1u << (2u * pin));
        }
        uint32_t edge = (d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u;
        if (edge != 0u && d->aux != NRT_NO_IRQ) {
          EXTI_EXTICR(pin / 4u) =
              (EXTI_EXTICR(pin / 4u) & ~(0xfu << (8u * (pin % 4u)))) | (port << (8u * (pin % 4u)));
          if (edge & 1u) {
            EXTI_FTSR1 |= 1u << pin;
          }
          if (edge & 2u) {
            EXTI_RTSR1 |= 1u << pin;
          }
          EXTI_RPR1 = 1u << pin;
          EXTI_FPR1 = 1u << pin;
          EXTI_IMR1 |= 1u << pin;
          uint32_t irq = exti_irq(pin);
          NVIC_ISER(irq / 32u) = 1u << (irq % 32u);
        }
        break;
      }
      case NRT_IO_UART:
        /* USART1 TX on PA9 (alternate function 7), 115200 Bd from 16 MHz. */
        RCC_APB2ENR |= 1u << 14; /* USART1 clock */
        GPIO_MODER(0u) = (GPIO_MODER(0u) & ~(3u << 18)) | (2u << 18);
        GPIO_AFRL(0u) = (GPIO_AFRL(0u) & ~(0xfu << 4)) | (7u << 4);
        USART1_BRR = (uint32_t)((F_CPU + 57600u) / 115200u);
        USART1_CR1 = (1u << 0) | (1u << 3); /* UE, TE */
        break;
      default:
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

uint32_t nrt_hal_now_ms(void) { return g_u5_ms; }

void nrt_hal_idle(void) { __asm volatile("wfi"); }

void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) { return ((GPIO_IDR(io->mask / 16u) >> (io->mask % 16u)) & 1u) != 0u; }

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) {
  uint32_t pin = io->mask % 16u;
  GPIO_BSRR(io->mask / 16u) = v ? (1u << pin) : (1u << (pin + 16u));
}

void nrt_hal_gpio_flip(const nrt_io_def *io) {
  nrt_crit_t s = nrt_crit_enter();
  GPIO_ODR(io->mask / 16u) ^= 1u << (io->mask % 16u);
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
  while ((USART1_ISR & (1u << 7)) == 0u) { /* TXE / TXFNF */
  }
  USART1_TDR = (uint32_t)(uint8_t)c;
}

/* ---- start-up ---------------------------------------------------------------------- */

extern uint32_t __data_load__;
extern uint32_t __data_start__;
extern uint32_t __data_end__;
extern uint32_t __bss_start__;
extern uint32_t __bss_end__;
extern uint32_t __stack_top;
int main(void);
void nrt_u5_reset(void);
void nrt_u5_unused(void);

void nrt_u5_reset(void) {
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

void nrt_u5_unused(void) {
  for (;;) {
  }
}

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Woverride-init"
__attribute__((section(".vectors"), used)) void (*const nrt_u5_vectors[16 + 125])(void) = {
    [0] = (void (*)(void))(uintptr_t)&__stack_top,
    [1] = nrt_u5_reset,
    [2 ... 16 + 124] = nrt_u5_unused,
    [15] = nrt_u5_systick,
    [16 + 11] = nrt_u5_exti,
    [16 + 12] = nrt_u5_exti,
    [16 + 13] = nrt_u5_exti,
    [16 + 14] = nrt_u5_exti,
    [16 + 15] = nrt_u5_exti,
    [16 + 16] = nrt_u5_exti,
    [16 + 17] = nrt_u5_exti,
    [16 + 18] = nrt_u5_exti,
    [16 + 19] = nrt_u5_exti,
    [16 + 20] = nrt_u5_exti,
    [16 + 21] = nrt_u5_exti,
    [16 + 22] = nrt_u5_exti,
    [16 + 23] = nrt_u5_exti,
    [16 + 24] = nrt_u5_exti,
    [16 + 25] = nrt_u5_exti,
    [16 + 26] = nrt_u5_exti,
};
#pragma GCC diagnostic pop
