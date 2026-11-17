/* neuron-rt: the Neuron runtime (task B3).
 *
 * Portable C99, no heap, no RTOS. A generated program describes itself with an `nrt_app`
 * (tables of tasks, interrupts and I/O) and calls `nrt_run`. The scheduler reproduces the
 * semantics of the reference interpreter (spec chapter 17): fixed priorities, run to
 * completion, logical execution time for signals, safe state on a fault.
 *
 * Ports: define NRT_PORT_AVR (microcontroller) or NRT_HOST (simulated time and a scenario,
 * used by the differential tests).
 */
#ifndef NRT_H
#define NRT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#if defined(NRT_PORT_AVR)
#include "nrt_port_avr.h"
#elif defined(NRT_HOST)
#include "nrt_port_host.h"
#else
#error "define NRT_PORT_AVR or NRT_HOST"
#endif

/* ---- faults -------------------------------------------------------------------- */

enum {
  NRT_F_NONE = 0,
  NRT_F_OVERFLOW = 1,
  NRT_F_DIVZERO = 2,
  NRT_F_CONTRACT = 3,
  NRT_F_STACK = 4,
  NRT_F_LOOP = 5,
  NRT_F_DEADLINE = 6,
  NRT_F_WATCHDOG = 7,
  NRT_F_USER = 16 /* user faults are numbered from here by the compiler */
};

/* ---- I/O descriptions ------------------------------------------------------------ */

enum { NRT_IO_GPIO_IN = 1, NRT_IO_GPIO_OUT = 2, NRT_IO_PWM = 3, NRT_IO_ADC = 4, NRT_IO_UART = 5 };

/* gpio `cfg` bits */
#define NRT_CFG_PULLUP 0x01u
#define NRT_CFG_EDGE_SHIFT 1 /* 2 bits: 0 none, 1 falling, 2 rising, 3 change */
#define NRT_NO_IRQ 255u

typedef struct nrt_io_def {
  uint8_t kind;
  const char *name;
  volatile uint8_t *port; /* NULL on the host */
  volatile uint8_t *ddr;
  volatile uint8_t *pin;
  uint8_t mask;
  uint8_t aux;  /* gpio in: interrupt number; adc: channel; pwm: timer*4+channel; uart: index */
  uint16_t cfg; /* gpio in: pull-up and edge bits; pwm: prescaler; uart: ubrr | 0x8000 for U2X */
} nrt_io_def;

/* ---- the application ------------------------------------------------------------- */

typedef void (*nrt_fn)(void);

typedef struct nrt_task_def {
  const char *name;
  uint32_t period_ms; /* 0 for a task released by an event */
  uint32_t offset_ms;
  uint8_t prio;
  uint8_t event;         /* 1-based event index, 0 for a periodic task */
  uint32_t deadline_ms;  /* signals written by the body become visible this long after the release; 0 = at once */
  nrt_fn snapshot;       /* copies the signals the task reads at its release (may be NULL) */
  nrt_fn body;
  nrt_fn commit;         /* makes the signals the task wrote visible (may be NULL) */
} nrt_task_def;

typedef struct nrt_app {
  const char *name;
  uint8_t n_io;
  const nrt_io_def *io;
  uint8_t n_tasks;
  const nrt_task_def *tasks;
  uint8_t n_irq;
  const nrt_fn *irq;     /* indexed by external interrupt number; entries may be NULL */
  uint8_t n_events;
  const char *const *event_names;
  uint8_t n_faults;      /* number of user faults */
  const char *const *fault_names;
  nrt_fn init;           /* initial values of variables and signals */
  nrt_fn safe_state;     /* writes the safe value of every output */
  nrt_fn safety_poll;    /* safety rules and the reset condition (may be NULL) */
  uint16_t safety_period_ms;
  void (*on_fault)(uint8_t code); /* may be NULL */
  uint16_t watchdog_ms;  /* 0 = no watchdog */
} nrt_app;

/* ---- core (portable) ----------------------------------------------------------------- */

void nrt_run(const nrt_app *app);
void nrt_core_init(const nrt_app *app);
void nrt_core_begin(uint32_t now_ms); /* commits due signals */
void nrt_core_end(uint32_t now_ms);   /* safety poll, releases, runs ready tasks */
void nrt_emit(uint8_t event);
void nrt_dispatch_irq(uint8_t n);
void nrt_fault(uint8_t code);
void nrt_reset(void);
bool nrt_is_faulted(void);
uint8_t nrt_fault_code(void);
uint32_t nrt_now(void);
const char *nrt_fault_name(uint8_t code);

/* I/O as the generated code uses it. While a fault is active, writes are dropped except
 * those of the safe state (property SAFE-1). */
bool nrt_gpio_get(uint8_t io);
void nrt_gpio_put(uint8_t io, bool v);
void nrt_gpio_flip(uint8_t io);
uint16_t nrt_adc_get(uint8_t io);
void nrt_pwm_put(uint8_t io, float duty);
void nrt_uart_text(uint8_t io, const char *s);
void nrt_uart_i32(uint8_t io, int32_t v);
void nrt_uart_f32(uint8_t io, float v);

/* ---- checked arithmetic ---------------------------------------------------------------
 * The compiler computes in 64 bits and checks the result against the type of the
 * expression, as the interpreter does (spec 03, 3.4). */
#define NRT_FIT(T, LO, HI, NAME)                                                          \
  static inline T nrt_fit_##NAME(int64_t v) {                                             \
    if (v < (int64_t)(LO) || v > (int64_t)(HI)) {                                         \
      nrt_fault(NRT_F_OVERFLOW);                                                          \
      return 0;                                                                           \
    }                                                                                     \
    return (T)v;                                                                          \
  }
NRT_FIT(uint8_t, 0, 255, u8)
NRT_FIT(int8_t, -128, 127, i8)
NRT_FIT(uint16_t, 0, 65535, u16)
NRT_FIT(int16_t, -32768, 32767, i16)
NRT_FIT(uint32_t, 0, 4294967295LL, u32)
NRT_FIT(int32_t, -2147483647LL - 1, 2147483647LL, i32)

static inline int64_t nrt_div(int64_t a, int64_t b) {
  if (b == 0) {
    nrt_fault(NRT_F_DIVZERO);
    return 0;
  }
  return a / b;
}

static inline int64_t nrt_rem(int64_t a, int64_t b) {
  if (b == 0) {
    nrt_fault(NRT_F_DIVZERO);
    return 0;
  }
  return a % b;
}

static inline float nrt_fdiv(float a, float b) {
  if (b == 0.0f) {
    nrt_fault(NRT_F_DIVZERO);
    return 0.0f;
  }
  return a / b;
}

static inline float nrt_fmin(float a, float b) { return a < b ? a : b; }
static inline float nrt_fmax(float a, float b) { return a > b ? a : b; }
static inline float nrt_fabs(float a) { return a < 0.0f ? -a : a; }
static inline float nrt_fclamp(float x, float lo, float hi) { return x < lo ? lo : (x > hi ? hi : x); }
static inline int64_t nrt_iabs(int64_t a) { return a < 0 ? -a : a; }
static inline int64_t nrt_imin(int64_t a, int64_t b) { return a < b ? a : b; }
static inline int64_t nrt_imax(int64_t a, int64_t b) { return a > b ? a : b; }
static inline int64_t nrt_iclamp(int64_t x, int64_t lo, int64_t hi) { return x < lo ? lo : (x > hi ? hi : x); }

/* Linear map of x from [a, b] to [c, d]. */
static inline float nrt_scale(float x, float a, float b, float c, float d) {
  if (b == a) {
    nrt_fault(NRT_F_DIVZERO);
    return c;
  }
  return c + (x - a) * (d - c) / (b - a);
}

/* ---- critical sections (statement-level atomicity for data shared with interrupts) ------- */
#define NRT_ATOMIC_BEGIN() { nrt_crit_t nrt_cs_ = nrt_crit_enter();
#define NRT_ATOMIC_END() nrt_crit_leave(nrt_cs_); }

/* Atomic loads of data that an interrupt also writes (a multi-byte read can tear on 8-bit parts). */
#define NRT_LD(T, NAME)                                                                   \
  static inline T nrt_ld_##NAME(const volatile T *p) {                                    \
    nrt_crit_t s = nrt_crit_enter();                                                      \
    T v = *p;                                                                             \
    nrt_crit_leave(s);                                                                    \
    return v;                                                                             \
  }
NRT_LD(bool, b)
NRT_LD(uint8_t, u8)
NRT_LD(int8_t, i8)
NRT_LD(uint16_t, u16)
NRT_LD(int16_t, i16)
NRT_LD(uint32_t, u32)
NRT_LD(int32_t, i32)
NRT_LD(float, f32)

/* ---- port interface (implemented in port/<name>) ------------------------------------- */
void nrt_hal_init(const nrt_app *app);
void nrt_hal_start(void);
uint32_t nrt_hal_now_ms(void);
void nrt_hal_idle(void);
void nrt_hal_wdt_feed(void);
bool nrt_hal_gpio_get(const nrt_io_def *io);
void nrt_hal_gpio_put(const nrt_io_def *io, bool v);
void nrt_hal_gpio_flip(const nrt_io_def *io);
uint16_t nrt_hal_adc_get(const nrt_io_def *io);
void nrt_hal_pwm_put(const nrt_io_def *io, float duty);
void nrt_hal_uart_put(const nrt_io_def *io, char c);

/* ---- trace hooks (host only; no code on the target) ---------------------------------- */
#if defined(NRT_HOST)
void nrt_trace(const char *kind, const char *name, const char *value);
void nrt_trace_sig_i(const char *name, int64_t v);
void nrt_trace_sig_b(const char *name, bool v);
#define NRT_TRACE(kind, name, value) nrt_trace((kind), (name), (value))
#define NRT_TRACE_SIG_I(name, v) nrt_trace_sig_i((name), (int64_t)(v))
#define NRT_TRACE_SIG_B(name, v) nrt_trace_sig_b((name), (v))
#else
#define NRT_TRACE(kind, name, value) ((void)0)
#define NRT_TRACE_SIG_I(name, v) ((void)0)
#define NRT_TRACE_SIG_B(name, v) ((void)0)
#endif

#endif /* NRT_H */
