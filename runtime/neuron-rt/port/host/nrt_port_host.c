/* neuron-rt host port.
 *
 * `app_host [--for <ms>] [scenario]` runs the program in simulated time, one millisecond at a
 * time, and prints the trace in the format of `neuron run`:
 *
 *       500.000 ms  OUT    led = true
 *
 * The scenario has the syntax of the interpreter (spec 17.2): `at 100 ms: button = true`.
 * Only whole milliseconds, booleans and integers are read.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "nrt.h"

#define HOST_MAX_IO 32
#define HOST_MAX_STEPS 512

static const nrt_app *g_app;
static uint32_t g_ms;
static bool g_out[HOST_MAX_IO];
static bool g_in[HOST_MAX_IO];
static bool g_written[HOST_MAX_IO];
static int32_t g_adc[HOST_MAX_IO];
static char g_line[HOST_MAX_IO][96];
static unsigned g_line_len[HOST_MAX_IO];

static int io_index(const nrt_io_def *io) { return (int)(io - g_app->io); }

void nrt_trace(const char *kind, const char *name, const char *value) {
  char t[32];
  snprintf(t, sizeof t, "%u.000 ms", (unsigned)g_ms);
  if (value == NULL || value[0] == '\0') {
    printf("%14s  %-5s  %s\n", t, kind, name);
  } else {
    printf("%14s  %-5s  %s = %s\n", t, kind, name, value);
  }
}

void nrt_trace_sig_i(const char *name, int64_t v) {
  char buf[32];
  snprintf(buf, sizeof buf, "%lld", (long long)v);
  nrt_trace("SIG", name, buf);
}

void nrt_trace_sig_b(const char *name, bool v) { nrt_trace("SIG", name, v ? "true" : "false"); }

void nrt_hal_init(const nrt_app *app) {
  g_app = app;
  memset(g_out, 0, sizeof g_out);
  memset(g_in, 0, sizeof g_in);
  memset(g_written, 0, sizeof g_written);
  memset(g_adc, 0, sizeof g_adc);
  memset(g_line_len, 0, sizeof g_line_len);
}

void nrt_hal_start(void) {}
uint32_t nrt_hal_now_ms(void) { return g_ms; }
void nrt_hal_idle(void) {}
void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) {
  int i = io_index(io);
  return io->kind == NRT_IO_GPIO_IN ? g_in[i] : g_out[i];
}

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) {
  /* Like the interpreter: the first write of an output is traced, then only changes. */
  int i = io_index(io);
  bool first = !g_written[i];
  g_written[i] = true;
  if (first || g_out[i] != v) {
    g_out[i] = v;
    nrt_trace("OUT", io->name, v ? "true" : "false");
  }
}

void nrt_hal_gpio_flip(const nrt_io_def *io) { nrt_hal_gpio_put(io, !g_out[io_index(io)]); }

uint16_t nrt_hal_adc_get(const nrt_io_def *io) { return (uint16_t)g_adc[io_index(io)]; }

void nrt_hal_pwm_put(const nrt_io_def *io, float duty) {
  char name[64];
  char val[32];
  snprintf(name, sizeof name, "%s.duty", io->name);
  snprintf(val, sizeof val, "%g", (double)duty);
  nrt_trace("OUT", name, val);
}

void nrt_hal_uart_put(const nrt_io_def *io, char c) {
  int i = io_index(io);
  if (c == '\n' || g_line_len[i] + 1 >= sizeof g_line[i]) {
    char name[64];
    g_line[i][g_line_len[i] > 1 ? g_line_len[i] - 1 : 0] = '\0';
    snprintf(name, sizeof name, "%s.print", io->name);
    nrt_trace("OUT", name, g_line[i]);
    g_line_len[i] = 0;
    return;
  }
  if (g_line_len[i] == 0) {
    g_line_len[i] = 1;
  }
  g_line[i][g_line_len[i] - 1] = c;
  g_line_len[i]++;
}

/* ---- scenario ------------------------------------------------------------------------- */

typedef struct {
  uint32_t t;
  int io;
  int32_t value;
} step_t;

static step_t g_steps[HOST_MAX_STEPS];
static int g_n_steps;

static int find_io(const char *name) {
  for (int i = 0; i < g_app->n_io; i++) {
    if (strcmp(g_app->io[i].name, name) == 0) {
      return i;
    }
  }
  return -1;
}

static char *trim(char *s) {
  while (*s == ' ' || *s == '\t') s++;
  char *e = s + strlen(s);
  while (e > s && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\n' || e[-1] == '\r')) *--e = '\0';
  return s;
}

static void load_scenario(const char *path) {
  FILE *f = fopen(path, "r");
  if (f == NULL) {
    fprintf(stderr, "cannot open scenario %s\n", path);
    exit(2);
  }
  char buf[256];
  while (fgets(buf, sizeof buf, f) != NULL && g_n_steps < HOST_MAX_STEPS) {
    char *hash = strchr(buf, '#');
    if (hash != NULL) *hash = '\0';
    char *line = trim(buf);
    if (*line == '\0') continue;
    if (strncmp(line, "at ", 3) == 0) line = trim(line + 3);
    char *colon = strchr(line, ':');
    if (colon == NULL) continue;
    *colon = '\0';
    char *when = trim(line);
    char *rest = trim(colon + 1);
    double amount = 0;
    char unit[8] = "";
    if (sscanf(when, "%lf %7s", &amount, unit) < 1) continue;
    uint32_t t = (uint32_t)(strcmp(unit, "s") == 0 ? amount * 1000.0 + 0.5 : amount + 0.5);
    char *eq = strchr(rest, '=');
    if (eq == NULL) continue;
    *eq = '\0';
    char *name = trim(rest);
    char *val = trim(eq + 1);
    int io = find_io(name);
    if (io < 0) continue;
    int32_t v = 0;
    if (strcmp(val, "true") == 0) v = 1;
    else if (strcmp(val, "false") == 0) v = 0;
    else v = (int32_t)atol(val);
    g_steps[g_n_steps].t = t;
    g_steps[g_n_steps].io = io;
    g_steps[g_n_steps].value = v;
    g_n_steps++;
  }
  fclose(f);
}

static void apply_step(const step_t *s) {
  const nrt_io_def *d = &g_app->io[s->io];
  if (d->kind == NRT_IO_ADC) {
    g_adc[s->io] = s->value;
    return;
  }
  if (d->kind != NRT_IO_GPIO_IN) {
    return;
  }
  bool prev = g_in[s->io];
  bool now = s->value != 0;
  g_in[s->io] = now;
  unsigned edge = (d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u;
  bool fire = (edge == 1 && prev && !now) || (edge == 2 && !prev && now) || (edge == 3 && prev != now);
  if (fire && d->aux != NRT_NO_IRQ) {
    nrt_dispatch_irq(d->aux);
  }
}

int nrt_main(const nrt_app *app, int argc, char **argv) {
  uint32_t end_ms = 1000;
  const char *scenario = NULL;
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--for") == 0 && i + 1 < argc) {
      end_ms = (uint32_t)atol(argv[++i]);
    } else {
      scenario = argv[i];
    }
  }
  g_app = app;
  nrt_core_init(app);
  if (scenario != NULL) {
    load_scenario(scenario);
  }
  for (uint32_t t = 0; t <= end_ms; t++) {
    g_ms = t;
    nrt_core_begin(t);
    for (int i = 0; i < g_n_steps; i++) {
      if (g_steps[i].t == t) {
        apply_step(&g_steps[i]);
      }
    }
    nrt_core_end(t);
  }
  return 0;
}
