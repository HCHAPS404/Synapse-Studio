/* neuron-rt port for AVR (ATmega328P, ATmega2560): critical sections and pin registers. */
#ifndef NRT_PORT_AVR_H
#define NRT_PORT_AVR_H

#include <avr/interrupt.h>
#include <avr/io.h>
#include <stdint.h>

typedef uint8_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) {
  uint8_t s = SREG;
  cli();
  return s;
}

static inline void nrt_crit_leave(nrt_crit_t s) { SREG = s; }

/* Registers of a pin as the I/O table wants them. */
#define NRT_PIN(port, ddr, pin, bit) &(port), &(ddr), &(pin), (uint8_t)(1u << (bit))

#endif /* NRT_PORT_AVR_H */
