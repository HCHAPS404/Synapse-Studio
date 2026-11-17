/* neuron-rt port for AVR: 1 kHz tick on timer0, GPIO, external interrupts, 8-bit fast PWM on
 * timer1 and timer2, ADC, UART0 with a transmit ring buffer, and the watchdog. */
#include <avr/sleep.h>
#include <avr/wdt.h>

#include "nrt.h"

#ifndef F_CPU
#error "F_CPU must be defined (the compiler passes the system clock)"
#endif

/* ---- 1 ms tick on timer0 (CTC) --------------------------------------------------- */
#if ((F_CPU / 64UL / 1000UL) <= 256UL)
#define NRT_TICK_TOP (F_CPU / 64UL / 1000UL - 1UL)
#define NRT_TICK_CS ((1 << CS01) | (1 << CS00))
#else
#define NRT_TICK_TOP (F_CPU / 256UL / 1000UL - 1UL)
#define NRT_TICK_CS (1 << CS02)
#endif

static volatile uint32_t g_ticks;
static const nrt_app *g_app;

ISR(TIMER0_COMPA_vect) { g_ticks++; }

/* ---- external interrupts -------------------------------------------------------------- */
#define NRT_IRQ(n)                                                                        \
  ISR(INT##n##_vect) { nrt_dispatch_irq(n); }
#ifdef INT0_vect
NRT_IRQ(0)
#endif
#ifdef INT1_vect
NRT_IRQ(1)
#endif
#ifdef INT2_vect
NRT_IRQ(2)
#endif
#ifdef INT3_vect
NRT_IRQ(3)
#endif
#ifdef INT4_vect
NRT_IRQ(4)
#endif
#ifdef INT5_vect
NRT_IRQ(5)
#endif
#ifdef INT6_vect
NRT_IRQ(6)
#endif
#ifdef INT7_vect
NRT_IRQ(7)
#endif

static void irq_config(uint8_t n, uint8_t edge) {
  /* ISC bits: 1 = any change, 2 = falling, 3 = rising. */
  uint8_t isc = (edge == 1) ? 2u : (edge == 2) ? 3u : 1u;
  if (n < 4) {
    uint8_t sh = (uint8_t)(2u * n);
    EICRA = (uint8_t)((EICRA & (uint8_t)~(3u << sh)) | (uint8_t)(isc << sh));
  }
#ifdef EICRB
  else {
    uint8_t sh = (uint8_t)(2u * (n - 4u));
    EICRB = (uint8_t)((EICRB & (uint8_t)~(3u << sh)) | (uint8_t)(isc << sh));
  }
#endif
  EIFR = (uint8_t)(1u << n);
  EIMSK |= (uint8_t)(1u << n);
}

/* ---- UART0 with a transmit ring buffer --------------------------------------------------- */
#define NRT_TX_SIZE 64u
static volatile uint8_t g_tx[NRT_TX_SIZE];
static volatile uint8_t g_tx_head;
static volatile uint8_t g_tx_tail;

/* The vector is called USART_UDRE on the ATmega328P and USART0_UDRE on the ATmega2560. */
#if defined(USART_UDRE_vect)
#define NRT_UDRE_VECT USART_UDRE_vect
#else
#define NRT_UDRE_VECT USART0_UDRE_vect
#endif

ISR(NRT_UDRE_VECT) {
  if (g_tx_head == g_tx_tail) {
    UCSR0B &= (uint8_t)~(1 << UDRIE0);
  } else {
    UDR0 = g_tx[g_tx_tail];
    g_tx_tail = (uint8_t)((g_tx_tail + 1u) % NRT_TX_SIZE);
  }
}

void nrt_hal_uart_put(const nrt_io_def *io, char c) {
  (void)io;
  uint8_t next = (uint8_t)((g_tx_head + 1u) % NRT_TX_SIZE);
  if (next == g_tx_tail) {
    return; /* full: the character is dropped, a print never blocks (spec 07 7.2) */
  }
  g_tx[g_tx_head] = (uint8_t)c;
  g_tx_head = next;
  UCSR0B |= (1 << UDRIE0);
}

/* ---- PWM: 8-bit fast PWM, timer1 (channels a, b) and timer2 (channels a, b) --------------- */
static uint8_t t1_cs(uint16_t prescaler) {
  switch (prescaler) {
    case 1: return (1 << CS10);
    case 8: return (1 << CS11);
    case 64: return (1 << CS11) | (1 << CS10);
    case 256: return (1 << CS12);
    default: return (1 << CS12) | (1 << CS10);
  }
}

static uint8_t t2_cs(uint16_t prescaler) {
  switch (prescaler) {
    case 1: return (1 << CS20);
    case 8: return (1 << CS21);
    case 32: return (1 << CS21) | (1 << CS20);
    case 64: return (1 << CS22);
    case 128: return (1 << CS22) | (1 << CS20);
    case 256: return (1 << CS22) | (1 << CS21);
    default: return (1 << CS22) | (1 << CS21) | (1 << CS20);
  }
}

static void pwm_config(const nrt_io_def *io) {
  uint8_t timer = (uint8_t)(io->aux >> 2);
  uint8_t chan = (uint8_t)(io->aux & 3u);
  if (timer == 1) {
    TCCR1A |= (uint8_t)((chan == 0 ? (1 << COM1A1) : (1 << COM1B1)) | (1 << WGM10));
    TCCR1B = (uint8_t)((1 << WGM12) | t1_cs(io->cfg));
  } else if (timer == 2) {
    TCCR2A |= (uint8_t)((chan == 0 ? (1 << COM2A1) : (1 << COM2B1)) | (1 << WGM21) | (1 << WGM20));
    TCCR2B = t2_cs(io->cfg);
  }
}

void nrt_hal_pwm_put(const nrt_io_def *io, float duty) {
  uint8_t timer = (uint8_t)(io->aux >> 2);
  uint8_t chan = (uint8_t)(io->aux & 3u);
  uint8_t v = (uint8_t)(duty * 255.0f + 0.5f);
  if (timer == 1) {
    if (chan == 0) {
      OCR1A = v;
    } else {
      OCR1B = v;
    }
  } else if (timer == 2) {
    if (chan == 0) {
      OCR2A = v;
    } else {
      OCR2B = v;
    }
  }
}

/* ---- ADC -------------------------------------------------------------------------------- */
uint16_t nrt_hal_adc_get(const nrt_io_def *io) {
  uint8_t ch = io->aux;
#ifdef MUX5
  if (ch >= 8) {
    ADCSRB |= (1 << MUX5);
  } else {
    ADCSRB &= (uint8_t)~(1 << MUX5);
  }
#endif
  ADMUX = (uint8_t)((1 << REFS0) | (ch & 7u));
  ADCSRA |= (1 << ADSC);
  while (ADCSRA & (1 << ADSC)) {
  }
  return ADC;
}

/* ---- GPIO ------------------------------------------------------------------------------- */
bool nrt_hal_gpio_get(const nrt_io_def *io) { return (*io->pin & io->mask) != 0; }

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) {
  NRT_ATOMIC_BEGIN()
  if (v) {
    *io->port |= io->mask;
  } else {
    *io->port &= (uint8_t)~io->mask;
  }
  NRT_ATOMIC_END()
}

void nrt_hal_gpio_flip(const nrt_io_def *io) { *io->pin = io->mask; /* writing 1 to PINx toggles PORTx */ }

/* ---- init, start, time ----------------------------------------------------------------- */
void nrt_hal_init(const nrt_app *app) {
  g_app = app;
  TCCR0A = (1 << WGM01);
  TCCR0B = NRT_TICK_CS;
  OCR0A = (uint8_t)NRT_TICK_TOP;
  TIMSK0 = (1 << OCIE0A);
  bool adc_on = false;
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        *d->ddr |= d->mask;
        *d->port &= (uint8_t)~d->mask;
        break;
      case NRT_IO_GPIO_IN:
        *d->ddr &= (uint8_t)~d->mask;
        if (d->cfg & NRT_CFG_PULLUP) {
          *d->port |= d->mask;
        } else {
          *d->port &= (uint8_t)~d->mask;
        }
        if (d->aux != NRT_NO_IRQ && ((d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u) != 0) {
          irq_config(d->aux, (uint8_t)((d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u));
        }
        break;
      case NRT_IO_PWM:
        *d->ddr |= d->mask;
        pwm_config(d);
        break;
      case NRT_IO_ADC:
        adc_on = true;
        break;
      case NRT_IO_UART:
        UBRR0 = (uint16_t)(d->cfg & 0x0FFFu);
        if (d->cfg & 0x8000u) {
          UCSR0A |= (1 << U2X0);
        }
        UCSR0B = (1 << TXEN0);
        UCSR0C = (1 << UCSZ01) | (1 << UCSZ00);
        break;
      default:
        break;
    }
  }
  if (adc_on) {
    ADMUX = (1 << REFS0);
    ADCSRA = (1 << ADEN) | (1 << ADPS2) | (1 << ADPS1) | (1 << ADPS0);
  }
}

static uint8_t wdt_code(uint16_t ms) {
  if (ms <= 15) return WDTO_15MS;
  if (ms <= 30) return WDTO_30MS;
  if (ms <= 60) return WDTO_60MS;
  if (ms <= 120) return WDTO_120MS;
  if (ms <= 250) return WDTO_250MS;
  if (ms <= 500) return WDTO_500MS;
  if (ms <= 1000) return WDTO_1S;
  if (ms <= 2000) return WDTO_2S;
  if (ms <= 4000) return WDTO_4S;
  return WDTO_8S;
}

void nrt_hal_start(void) {
  if (g_app != NULL && g_app->watchdog_ms != 0) {
    wdt_enable(wdt_code(g_app->watchdog_ms));
  }
  sei();
}

uint32_t nrt_hal_now_ms(void) {
  uint32_t t;
  NRT_ATOMIC_BEGIN()
  t = g_ticks;
  NRT_ATOMIC_END()
  return t;
}

void nrt_hal_idle(void) {
  set_sleep_mode(SLEEP_MODE_IDLE);
  sleep_mode();
}

void nrt_hal_wdt_feed(void) {
  if (g_app != NULL && g_app->watchdog_ms != 0) {
    wdt_reset();
  }
}
