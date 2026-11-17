/* neuron-rt host port, display bus: every I2C write and SPI transfer is printed as a BUS
 * line, so the tests can rebuild the image the panel would show (crates/neuron-ui, emu.rs). */
#include <stdio.h>

#include "nrt_ui.h"

static void bus_line(const char *head, const uint8_t *d, uint16_t n) {
  printf("%10u.000 ms  BUS    %s:", (unsigned)nrt_now(), head);
  for (uint16_t i = 0; i < n; i++) {
    printf(" %02x", d[i]);
  }
  printf("\n");
}

void nrt_hal_bus_init(uint8_t driver) { (void)driver; }

void nrt_hal_i2c_write(uint8_t addr, const uint8_t *data, uint16_t n) {
  char head[16];
  snprintf(head, sizeof head, "i2c %02x", addr);
  bus_line(head, data, n);
}

void nrt_hal_spi_write(bool is_data, const uint8_t *data, uint16_t n) { bus_line(is_data ? "spi d" : "spi c", data, n); }

void nrt_hal_delay_ms(uint16_t ms) { (void)ms; }
