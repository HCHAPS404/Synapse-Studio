/* neuron-rt port for the nRF52840 (Arduino Nano 33 BLE Sense Rev2), bare metal, Cortex-M4. */
#ifndef NRT_PORT_NRF52_H
#define NRT_PORT_NRF52_H

#include <stdint.h>

typedef uint32_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) {
  uint32_t s;
  __asm volatile("mrs %0, primask\n\tcpsid i" : "=r"(s) : : "memory");
  return s;
}

static inline void nrt_crit_leave(nrt_crit_t s) { __asm volatile("msr primask, %0" : : "r"(s) : "memory"); }

/* The nRF52840 has two 32-bit GPIO ports (P0 pins 0..31, P1 pins 0..15). The I/O table keeps
 * `port * 32 + bit` in `mask` (uint8_t, so P1.15 = 47 fits). */
enum { NRT_PORT0 = 0, NRT_PORT1 = 1 };
#define NRT_PIN(port, ddr, pin, bit) NULL, NULL, NULL, (uint8_t)(NRT_##port * 32u + (bit))

#endif /* NRT_PORT_NRF52_H */
