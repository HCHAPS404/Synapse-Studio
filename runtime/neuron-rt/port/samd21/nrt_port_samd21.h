/* neuron-rt port for the SAMD21G18A (Arduino Nano 33 IoT), bare metal, Cortex-M0+. */
#ifndef NRT_PORT_SAMD21_H
#define NRT_PORT_SAMD21_H

#include <stdint.h>

typedef uint32_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) {
  uint32_t s;
  __asm volatile("mrs %0, primask\n\tcpsid i" : "=r"(s) : : "memory");
  return s;
}

static inline void nrt_crit_leave(nrt_crit_t s) { __asm volatile("msr primask, %0" : : "r"(s) : "memory"); }

/* The SAMD21 PORT peripheral has two 32-bit groups: group 0 = PA, group 1 = PB. The I/O table
 * keeps `group * 32 + pin` in `mask`. */
enum { NRT_PORTA = 0, NRT_PORTB = 1 };
#define NRT_PIN(port, ddr, pin, bit) NULL, NULL, NULL, (uint8_t)(NRT_##port * 32u + (bit))

#endif /* NRT_PORT_SAMD21_H */
