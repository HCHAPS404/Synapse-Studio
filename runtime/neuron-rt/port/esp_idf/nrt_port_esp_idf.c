/* neuron-rt port for the ESP32-S3 on ESP-IDF: GPIO through the gpio driver, edge interrupts
 * through the GPIO ISR service, a 1 ms tick from esp_timer, UART0 through the uart driver.
 * The scheduler of nrt_core runs in the main task; the tick wakes it with a notification.
 * This port is written against the public ESP-IDF API (v5); it is built with `idf.py build`.
 */
#include "nrt.h"

#include "driver/gpio.h"
#include "driver/uart.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static const nrt_app *g_esp_app;
static volatile uint32_t g_esp_ms;
static TaskHandle_t g_esp_main;
static portMUX_TYPE g_esp_mux = portMUX_INITIALIZER_UNLOCKED;

nrt_crit_t nrt_crit_enter(void) {
  taskENTER_CRITICAL(&g_esp_mux);
  return 0u;
}

void nrt_crit_leave(nrt_crit_t s) {
  (void)s;
  taskEXIT_CRITICAL(&g_esp_mux);
}

static void esp_tick(void *arg) {
  (void)arg;
  g_esp_ms++;
  if (g_esp_main != NULL) {
    xTaskNotifyGive(g_esp_main);
  }
}

static void esp_gpio_isr(void *arg) {
  nrt_dispatch_irq((uint8_t)(uintptr_t)arg);
}

void nrt_hal_init(const nrt_app *app) {
  g_esp_app = app;
  g_esp_main = xTaskGetCurrentTaskHandle();
  bool isr = false;
  for (uint8_t i = 0; i < app->n_io; i++) {
    const nrt_io_def *d = &app->io[i];
    gpio_num_t n = (gpio_num_t)d->mask;
    switch (d->kind) {
      case NRT_IO_GPIO_OUT:
        gpio_reset_pin(n);
        gpio_set_direction(n, GPIO_MODE_OUTPUT);
        gpio_set_level(n, 0);
        break;
      case NRT_IO_GPIO_IN: {
        gpio_reset_pin(n);
        gpio_set_direction(n, GPIO_MODE_INPUT);
        gpio_set_pull_mode(n, (d->cfg & NRT_CFG_PULLUP) ? GPIO_PULLUP_ONLY : GPIO_FLOATING);
        uint32_t edge = (d->cfg >> NRT_CFG_EDGE_SHIFT) & 3u;
        if (edge != 0u && d->aux != NRT_NO_IRQ) {
          if (!isr) {
            gpio_install_isr_service(0);
            isr = true;
          }
          gpio_set_intr_type(n, edge == 1u ? GPIO_INTR_NEGEDGE : (edge == 2u ? GPIO_INTR_POSEDGE : GPIO_INTR_ANYEDGE));
          gpio_isr_handler_add(n, esp_gpio_isr, (void *)(uintptr_t)d->aux);
        }
        break;
      }
      case NRT_IO_UART:
        uart_driver_install(UART_NUM_0, 256, 256, 0, NULL, 0);
        break;
      default:
        break;
    }
  }
}

void nrt_hal_start(void) {
  const esp_timer_create_args_t args = {.callback = esp_tick, .name = "nrt_tick"};
  esp_timer_handle_t t;
  esp_timer_create(&args, &t);
  esp_timer_start_periodic(t, 1000);
}

uint32_t nrt_hal_now_ms(void) { return g_esp_ms; }

void nrt_hal_idle(void) { (void)ulTaskNotifyTake(pdTRUE, pdMS_TO_TICKS(10)); }

void nrt_hal_wdt_feed(void) {}

bool nrt_hal_gpio_get(const nrt_io_def *io) { return gpio_get_level((gpio_num_t)io->mask) != 0; }

void nrt_hal_gpio_put(const nrt_io_def *io, bool v) { gpio_set_level((gpio_num_t)io->mask, v ? 1 : 0); }

void nrt_hal_gpio_flip(const nrt_io_def *io) {
  gpio_num_t n = (gpio_num_t)io->mask;
  gpio_set_level(n, gpio_get_level(n) ? 0 : 1);
}

uint16_t nrt_hal_adc_get(const nrt_io_def *io) {
  (void)io;
  return 0u;
}

void nrt_hal_pwm_put(const nrt_io_def *io, float duty) {
  (void)io;
  (void)duty;
}

void nrt_hal_uart_put(const nrt_io_def *io, char c) {
  (void)io;
  uart_write_bytes(UART_NUM_0, &c, 1);
}

void app_main(void) { (void)nrt_generated_main(); }
