/* neuron-rt port for the STM32F4 family (NUCLEO-F446RE): start-up from flash, GPIO, EXTI edge
 * interrupts, the 1 ms tick from SysTick and USART2 transmit (the ST-LINK virtual COM port on
 * PA2). The core runs on the 16 MHz internal oscillator (HSI, the reset clock); the PLL is a
 * later improvement. Register addresses: STM32F446 reference manual (public document).
 */
#include "nrt.h"

#define REG(a) (*(volatile uint32_t *)(uintptr_t)(a))

#define RCC_AHB1ENR REG(0x40023830u)
#define RCC_APB1ENR REG(0x40023840u)
#define RCC_APB2ENR REG(0x40023844u)
#define GPIO_BASE(p) (0x40020000u + 0x400u * (p))
#define GPIO_MODER(p) REG(GPIO_BASE(p) + 0x00u)
#define GPIO_PUPDR(p) REG(GPIO_BASE(p) + 0x0cu)
#define GPIO_IDR(p) REG(GPIO_BASE(p) + 0x10u)
#define GPIO_ODR(p) REG(GPIO_BASE(p) + 0x14u)
#define GPIO_BSRR(p) REG(GPIO_BASE(p) + 0x18u)
#define GPIO_AFRL(p) REG(GPIO_BASE(p) + 0x20u)
#define SYSCFG_EXTICR(i) REG(0x40013808u + 4u * (i))
#define EXTI_IMR REG(0x40013c00u)
#define EXTI_RTSR REG(0x40013c08u)
#define EXTI_FTSR REG(0x40013c0cu)
#define EXTI_PR REG(0x40013c14u)
#define USART2_SR REG(0x40004400u)
#define USART2_DR REG(0x40004404u)
#define USART2_BRR REG(0x40004408u)
#define USART2_CR1 REG(0x4000440cu)
#define SYST_CSR REG(0xe000e010u)
#define SYST_RVR REG(0xe000e014u)
#define SYST_CVR REG(0xe000e018u)
#define NVIC_ISER(i) REG(0xe000e100u + 4u * (i))

#ifndef F_CPU
#define F_CPU 16000000u
#endif

static const nrt_app *g_st_app;
static volatile uint32_t g_st_ms;

void nrt_st_systick(void);
void nrt_st_exti(void);

void nrt_st_systick(void) { g_st_ms++; }

/* One handler for every EXTI line: the pending bits say which lines fired. */
void nrt_st_exti(void) {
  uint32_t pr = EXTI_PR;
  EXTI_PR = pr;
  if (g_st_app == NULL) {
    return;
  }
  for (uint8_t i = 0; i < g_st_app->n_io; i++) {
    const nrt_io_def *d = &g_st_app->io[i];
    if (d->kind == NRT_IO_GPIO_IN && d->aux != NRT_NO_IRQ && (pr & (1u << (d->mask % 16u))) != 0u) {
      nrt_dispatch_irq(d->aux);
    }
  }
}

/* NVIC number of the EXTI line. */
static uint32_t exti_irq(uint32_t line) {
  if (line <= 4u) {
    return 6u + line;
  }
  return line <= 9u ? 23u : 40u;
}

void nrt_hal_init(const nrt_app *app) {
  g_st_app = app;
  RCC_AHB1ENR |= 0xffu;      /* GPIOA..GPIOH clocks */
  RCC_APB2ENR |= 1u << 14;   /* SYSCFG */
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
          SYSCFG_EXTICR(pin / 4u) = (SYSCFG_EXTICR(pin / 4u) & ~(0xfu << (4u * (pin % 4u)))) | (port << (4u * (pin % 4u)));
          if (edge & 1u) {
            EXTI_FTSR |= 1u << pin;
          }
          if (edge & 2u) {
            EXTI_RTSR |= 1u << pin;
          }
          EXTI_PR = 1u << pin;
          EXTI_IMR |= 1u << pin;
          uint32_t irq = exti_irq(pin);
          NVIC_ISER(irq / 32u) = 1u << (irq % 32u);
        }
        break;
      }
      case NRT_IO_UART:
        /* USART2 TX on PA2 (alternate function 7), 115200 Bd from 16 MHz. */
        RCC_APB1ENR |= 1u << 17;
        GPIO_MODER(0u) = (GPIO_MODER(0u) & ~(3u << 4)) | (2u << 4);
        GPIO_AFRL(0u) = (GPIO_AFRL(0u) & ~(0xfu << 8)) | (7u << 8);
        USART2_BRR = (uint32_t)((F_CPU + 57600u) / 115200u);
        USART2_CR1 = (1u << 13) | (1u << 3); /* enable, transmitter */
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

uint32_t nrt_hal_now_ms(void) { return g_st_ms; }

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
  while ((USART2_SR & (1u << 7)) == 0u) { /* transmit register not empty */
  }
  USART2_DR = (uint32_t)(uint8_t)c;
}

/* ---- start-up ---------------------------------------------------------------------- */

extern uint32_t __data_load__;
extern uint32_t __data_start__;
extern uint32_t __data_end__;
extern uint32_t __bss_start__;
extern uint32_t __bss_end__;
extern uint32_t __stack_top;
int main(void);
void nrt_st_reset(void);
void nrt_st_unused(void);

void nrt_st_reset(void) {
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

void nrt_st_unused(void) {
  for (;;) {
  }
}

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Woverride-init"
__attribute__((section(".vectors"), used)) void (*const nrt_st_vectors[16 + 97])(void) = {
    [0] = (void (*)(void))(uintptr_t)&__stack_top,
    [1] = nrt_st_reset,
    [2 ... 16 + 96] = nrt_st_unused,
    [15] = nrt_st_systick,
    [16 + 6] = nrt_st_exti,
    [16 + 7] = nrt_st_exti,
    [16 + 8] = nrt_st_exti,
    [16 + 9] = nrt_st_exti,
    [16 + 10] = nrt_st_exti,
    [16 + 23] = nrt_st_exti,
    [16 + 40] = nrt_st_exti,
};
#pragma GCC diagnostic pop
