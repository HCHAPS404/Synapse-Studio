/* neuron-rt core: the scheduler, faults and the safe state (task B3).
 *
 * Order inside one millisecond, as in the interpreter (spec 17.4):
 *   1. signal commits   (nrt_core_begin)
 *   2. inputs and interrupts (done by the port, between begin and end)
 *   3. safety poll, releases of periodic tasks, then the ready tasks by priority (nrt_core_end)
 * Tasks run to completion. Equal priorities run in table order.
 */
#include "nrt.h"

#define NRT_MAX_TASKS 24

typedef struct {
  uint32_t next;       /* next release of a periodic task */
  uint32_t release;    /* time of the release being served */
  uint32_t commit_at;  /* when the signals of the finished run become visible */
  uint8_t pending;     /* releases waiting to run */
  bool commit_pending;
} nrt_task_state;

static const nrt_app *g_app;
static nrt_task_state g_ts[NRT_MAX_TASKS];
static uint32_t g_safety_next;
static volatile bool g_faulted;
static volatile uint8_t g_fault_code;
static volatile bool g_safe_ctx; /* true while the safe state or an `on fault` handler runs */
static uint32_t g_now;

bool nrt_is_faulted(void) { return g_faulted; }
uint8_t nrt_fault_code(void) { return g_fault_code; }
uint32_t nrt_now(void) { return g_now; }

const char *nrt_fault_name(uint8_t code) {
  switch (code) {
    case NRT_F_OVERFLOW: return "Overflow";
    case NRT_F_DIVZERO: return "DivideByZero";
    case NRT_F_CONTRACT: return "ContractViolation";
    case NRT_F_STACK: return "StackOverflow";
    case NRT_F_LOOP: return "LoopBound";
    case NRT_F_DEADLINE: return "DeadlineMiss";
    case NRT_F_WATCHDOG: return "Watchdog";
    default: break;
  }
  if (g_app != NULL && code >= NRT_F_USER && (uint8_t)(code - NRT_F_USER) < g_app->n_faults) {
    return g_app->fault_names[code - NRT_F_USER];
  }
  return "Fault";
}

void nrt_core_init(const nrt_app *app) {
  g_app = app;
  g_faulted = false;
  g_fault_code = NRT_F_NONE;
  g_safe_ctx = false;
  g_now = 0;
  g_safety_next = 0;
  for (uint8_t i = 0; i < app->n_tasks && i < NRT_MAX_TASKS; i++) {
    g_ts[i].next = app->tasks[i].offset_ms;
    g_ts[i].release = 0;
    g_ts[i].commit_at = 0;
    g_ts[i].pending = 0;
    g_ts[i].commit_pending = false;
  }
  nrt_hal_init(app);
  if (app->init != NULL) {
    app->init();
  }
}

void nrt_fault(uint8_t code) {
  if (g_faulted) {
    return;
  }
  g_faulted = true;
  g_fault_code = code;
  /* Every activity stops: the work that was waiting to run is dropped. */
  if (g_app != NULL) {
    for (uint8_t i = 0; i < g_app->n_tasks && i < NRT_MAX_TASKS; i++) {
      g_ts[i].pending = 0;
    }
  }
  NRT_TRACE("FAULT", nrt_fault_name(code), "");
  g_safe_ctx = true;
  if (g_app != NULL) {
    if (g_app->safe_state != NULL) {
      g_app->safe_state();
    }
    if (g_app->on_fault != NULL) {
      g_app->on_fault(code);
    }
  }
  g_safe_ctx = false;
}

void nrt_reset(void) {
  if (!g_faulted) {
    return;
  }
  g_faulted = false;
  g_fault_code = NRT_F_NONE;
  NRT_TRACE("RESET", "fault", "");
}

void nrt_emit(uint8_t event) {
  if (g_app == NULL || event == 0) {
    return;
  }
  if (g_app->event_names != NULL && event <= g_app->n_events) {
    NRT_TRACE("EVENT", g_app->event_names[event - 1], "");
  }
  if (g_faulted) {
    return;
  }
  for (uint8_t i = 0; i < g_app->n_tasks && i < NRT_MAX_TASKS; i++) {
    if (g_app->tasks[i].event == event) {
      NRT_ATOMIC_BEGIN()
      if (g_ts[i].pending < 255) {
        g_ts[i].pending++;
      }
      g_ts[i].release = g_now;
      NRT_ATOMIC_END()
    }
  }
}

void nrt_dispatch_irq(uint8_t n) {
  if (g_app == NULL || g_faulted || n >= g_app->n_irq) {
    return;
  }
  nrt_fn f = g_app->irq[n];
  if (f != NULL) {
    f();
  }
}

/* ---- I/O wrappers -------------------------------------------------------------------- */

static const nrt_io_def *io_at(uint8_t io) {
  return (g_app != NULL && io < g_app->n_io) ? &g_app->io[io] : NULL;
}

bool nrt_gpio_get(uint8_t io) {
  const nrt_io_def *d = io_at(io);
  return d != NULL && nrt_hal_gpio_get(d);
}

void nrt_gpio_put(uint8_t io, bool v) {
  const nrt_io_def *d = io_at(io);
  if (d == NULL || (g_faulted && !g_safe_ctx)) {
    return;
  }
  nrt_hal_gpio_put(d, v);
}

void nrt_gpio_flip(uint8_t io) {
  const nrt_io_def *d = io_at(io);
  if (d == NULL || (g_faulted && !g_safe_ctx)) {
    return;
  }
  nrt_hal_gpio_flip(d);
}

uint16_t nrt_adc_get(uint8_t io) {
  const nrt_io_def *d = io_at(io);
  return d != NULL ? nrt_hal_adc_get(d) : 0;
}

void nrt_pwm_put(uint8_t io, float duty) {
  const nrt_io_def *d = io_at(io);
  if (d == NULL || (g_faulted && !g_safe_ctx)) {
    return;
  }
  if (duty < 0.0f) {
    duty = 0.0f;
  }
  if (duty > 1.0f) {
    duty = 1.0f;
  }
  nrt_hal_pwm_put(d, duty);
}

void nrt_uart_text(uint8_t io, const char *s) {
  const nrt_io_def *d = io_at(io);
  if (d == NULL || (g_faulted && !g_safe_ctx)) {
    return;
  }
  while (*s != '\0') {
    nrt_hal_uart_put(d, *s++);
  }
}

void nrt_uart_i32(uint8_t io, int32_t v) {
  char buf[12];
  uint8_t n = 0;
  uint32_t u = (v < 0) ? (uint32_t)(-(int64_t)v) : (uint32_t)v;
  do {
    buf[n++] = (char)('0' + (u % 10u));
    u /= 10u;
  } while (u != 0u && n < 11);
  char out[13];
  uint8_t k = 0;
  if (v < 0) {
    out[k++] = '-';
  }
  while (n > 0) {
    out[k++] = buf[--n];
  }
  out[k] = '\0';
  nrt_uart_text(io, out);
}

void nrt_uart_f32(uint8_t io, float v) {
  /* Three decimals without printf (printf with floats costs kilobytes on small parts). */
  if (v < 0.0f) {
    nrt_uart_text(io, "-");
    v = -v;
  }
  if (v > 2147483.0f) {
    nrt_uart_text(io, "big");
    return;
  }
  int32_t whole = (int32_t)v;
  int32_t frac = (int32_t)((v - (float)whole) * 1000.0f + 0.5f);
  if (frac >= 1000) {
    whole++;
    frac -= 1000;
  }
  nrt_uart_i32(io, whole);
  char tail[6] = {'.', (char)('0' + frac / 100), (char)('0' + (frac / 10) % 10), (char)('0' + frac % 10), '\0', '\0'};
  nrt_uart_text(io, tail);
}

/* ---- scheduler ------------------------------------------------------------------------ */

static void commit_due(uint32_t now) {
  for (uint8_t i = 0; i < g_app->n_tasks && i < NRT_MAX_TASKS; i++) {
    if (g_ts[i].commit_pending && (int32_t)(now - g_ts[i].commit_at) >= 0) {
      g_ts[i].commit_pending = false;
      if (g_app->tasks[i].commit != NULL) {
        g_app->tasks[i].commit();
      }
    }
  }
}

void nrt_core_begin(uint32_t now) {
  g_now = now;
  commit_due(now);
}

void nrt_core_end(uint32_t now) {
  g_now = now;
  /* A release that falls while the program is in a fault is dropped, even when the safety
   * logic resets the fault at this same instant (the interpreter decides at release time). */
  bool was_faulted = g_faulted;
  if (g_app->safety_poll != NULL && g_app->safety_period_ms != 0 && (int32_t)(now - g_safety_next) >= 0) {
    g_safety_next = now + g_app->safety_period_ms;
    g_app->safety_poll();
  }
  if (g_faulted) {
    return;
  }
  for (uint8_t i = 0; i < g_app->n_tasks && i < NRT_MAX_TASKS; i++) {
    const nrt_task_def *t = &g_app->tasks[i];
    if (t->period_ms != 0 && (int32_t)(now - g_ts[i].next) >= 0) {
      g_ts[i].next += t->period_ms;
      if (was_faulted) {
        continue;
      }
      if (g_ts[i].pending == 255) {
        nrt_fault(NRT_F_DEADLINE);
        return;
      }
      g_ts[i].pending++;
      g_ts[i].release = now;
    }
  }
  for (;;) {
    int best = -1;
    for (uint8_t i = 0; i < g_app->n_tasks && i < NRT_MAX_TASKS; i++) {
      if (g_ts[i].pending != 0 && (best < 0 || g_app->tasks[i].prio > g_app->tasks[best].prio)) {
        best = i;
      }
    }
    if (best < 0 || g_faulted) {
      break;
    }
    const nrt_task_def *t = &g_app->tasks[best];
    NRT_ATOMIC_BEGIN()
    g_ts[best].pending--;
    NRT_ATOMIC_END()
    if (t->snapshot != NULL) {
      NRT_ATOMIC_BEGIN()
      t->snapshot();
      NRT_ATOMIC_END()
    }
    t->body();
    if (t->commit != NULL && !g_faulted) {
      if (t->deadline_ms == 0) {
        t->commit();
      } else {
        g_ts[best].commit_at = g_ts[best].release + t->deadline_ms;
        g_ts[best].commit_pending = true;
      }
    }
  }
}

/* `nrt_run` is the entry point on the target: the port provides the tick. */
void nrt_run(const nrt_app *app) {
#if defined(NRT_PORT_AVR)
  nrt_core_init(app);
  nrt_hal_start();
  uint32_t last = 0;
  nrt_core_begin(0);
  nrt_core_end(0);
  for (;;) {
    uint32_t now = nrt_hal_now_ms();
    while ((int32_t)(now - last) > 0) {
      last++;
      nrt_core_begin(last);
      nrt_core_end(last);
      nrt_hal_wdt_feed();
    }
    nrt_hal_wdt_feed();
    nrt_hal_idle();
  }
#else
  (void)app; /* the host port has its own driver (nrt_host_run) */
#endif
}
