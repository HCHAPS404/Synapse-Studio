/* neuron-rt AVR port, display bus: the TWI master (I2C, 100 kHz, SDA/SCL on A4/A5 of the
 * UNO) and the SPI master (MOSI D11, SCK D13, chip select D10, data/command D9 on the UNO).
 * Registers: ATmega328P / ATmega2560 datasheets (public documents). */
#include <avr/io.h>
#include <util/delay.h>

#include "nrt_ui.h"

#if defined(__AVR_ATmega2560__)
#define SPI_DDR DDRB
#define SPI_PORT PORTB
#define SPI_SS PB0
#define SPI_SCK PB1
#define SPI_MOSI PB2
#define TFT_DC PB4 /* D10 */
#else
#define SPI_DDR DDRB
#define SPI_PORT PORTB
#define SPI_SS PB2   /* D10 */
#define SPI_SCK PB5  /* D13 */
#define SPI_MOSI PB3 /* D11 */
#define TFT_DC PB1   /* D9 */
#endif

static void twi_wait(void) {
  while ((TWCR & (1 << TWINT)) == 0) {
  }
}

void nrt_hal_bus_init(uint8_t driver) {
  if (driver == NRT_UI_SSD1306 || driver == NRT_UI_HD44780) {
    TWSR = 0;
    TWBR = (uint8_t)((F_CPU / 100000UL - 16UL) / 2UL);
    TWCR = (uint8_t)(1 << TWEN);
  } else {
    SPI_DDR |= (uint8_t)((1 << SPI_SS) | (1 << SPI_SCK) | (1 << SPI_MOSI) | (1 << TFT_DC));
    SPI_PORT |= (uint8_t)(1 << SPI_SS);
    SPCR = (uint8_t)((1 << SPE) | (1 << MSTR));
    SPSR = (uint8_t)(1 << SPI2X);
  }
}

void nrt_hal_i2c_write(uint8_t addr, const uint8_t *data, uint16_t n) {
  TWCR = (uint8_t)((1 << TWINT) | (1 << TWSTA) | (1 << TWEN));
  twi_wait();
  TWDR = (uint8_t)(addr << 1);
  TWCR = (uint8_t)((1 << TWINT) | (1 << TWEN));
  twi_wait();
  for (uint16_t i = 0; i < n; i++) {
    TWDR = data[i];
    TWCR = (uint8_t)((1 << TWINT) | (1 << TWEN));
    twi_wait();
  }
  TWCR = (uint8_t)((1 << TWINT) | (1 << TWSTO) | (1 << TWEN));
}

void nrt_hal_spi_write(bool is_data, const uint8_t *data, uint16_t n) {
  if (is_data) {
    SPI_PORT |= (uint8_t)(1 << TFT_DC);
  } else {
    SPI_PORT &= (uint8_t)~(1 << TFT_DC);
  }
  SPI_PORT &= (uint8_t)~(1 << SPI_SS);
  for (uint16_t i = 0; i < n; i++) {
    SPDR = data[i];
    while ((SPSR & (1 << SPIF)) == 0) {
    }
  }
  SPI_PORT |= (uint8_t)(1 << SPI_SS);
}

void nrt_hal_delay_ms(uint16_t ms) {
  while (ms-- > 0u) {
    _delay_ms(1);
  }
}
