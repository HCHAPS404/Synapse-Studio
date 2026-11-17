/* neuron-rt ESP-IDF port, display bus: I2C master on I2C_NUM_0 (SDA GPIO8, SCL GPIO9) and an
 * SPI device on SPI2 (MOSI GPIO11, SCLK GPIO12, CS GPIO10, data/command GPIO13), with the
 * public ESP-IDF v5 driver API. Built with `idf.py build`. */
#include "nrt_ui.h"

#include "driver/gpio.h"
#include "driver/i2c.h"
#include "driver/spi_master.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define NRT_I2C_SDA 8
#define NRT_I2C_SCL 9
#define NRT_SPI_MOSI 11
#define NRT_SPI_SCLK 12
#define NRT_SPI_CS 10
#define NRT_TFT_DC 13

static spi_device_handle_t g_spi;

void nrt_hal_bus_init(uint8_t driver) {
  if (driver == NRT_UI_SSD1306 || driver == NRT_UI_HD44780) {
    i2c_config_t c = {
        .mode = I2C_MODE_MASTER,
        .sda_io_num = NRT_I2C_SDA,
        .scl_io_num = NRT_I2C_SCL,
        .sda_pullup_en = GPIO_PULLUP_ENABLE,
        .scl_pullup_en = GPIO_PULLUP_ENABLE,
        .master.clk_speed = 400000,
    };
    i2c_param_config(I2C_NUM_0, &c);
    i2c_driver_install(I2C_NUM_0, I2C_MODE_MASTER, 0, 0, 0);
  } else {
    spi_bus_config_t bus = {
        .mosi_io_num = NRT_SPI_MOSI,
        .miso_io_num = -1,
        .sclk_io_num = NRT_SPI_SCLK,
        .quadwp_io_num = -1,
        .quadhd_io_num = -1,
        .max_transfer_sz = 4096,
    };
    spi_device_interface_config_t dev = {
        .clock_speed_hz = 20000000,
        .mode = 0,
        .spics_io_num = NRT_SPI_CS,
        .queue_size = 1,
    };
    spi_bus_initialize(SPI2_HOST, &bus, SPI_DMA_CH_AUTO);
    spi_bus_add_device(SPI2_HOST, &dev, &g_spi);
    gpio_reset_pin(NRT_TFT_DC);
    gpio_set_direction(NRT_TFT_DC, GPIO_MODE_OUTPUT);
  }
}

void nrt_hal_i2c_write(uint8_t addr, const uint8_t *data, uint16_t n) {
  i2c_master_write_to_device(I2C_NUM_0, addr, data, n, pdMS_TO_TICKS(50));
}

void nrt_hal_spi_write(bool is_data, const uint8_t *data, uint16_t n) {
  gpio_set_level(NRT_TFT_DC, is_data ? 1 : 0);
  spi_transaction_t t = {.length = (size_t)n * 8u, .tx_buffer = data};
  spi_device_polling_transmit(g_spi, &t);
}

void nrt_hal_delay_ms(uint16_t ms) { vTaskDelay(pdMS_TO_TICKS(ms)); }
