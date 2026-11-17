/* neuron-rt port for the Raspberry Pi RP2040 (Cortex-M0+) and RP2350 (Cortex-M33), bare metal.
 *
 * Define NRT_PORT_RP2040 or NRT_PORT_RP2350. The image runs from SRAM (loaded as a RAM UF2 by
 * the boot ROM, so no second-stage flash loader is needed). Register addresses come from the
 * public RP2040 and RP2350 datasheets; no SDK code is used.
 */
#ifndef NRT_PORT_RP_H
#define NRT_PORT_RP_H

#include <stdint.h>

typedef uint32_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) {
  uint32_t s;
  __asm volatile("mrs %0, primask\n\tcpsid i" : "=r"(s) : : "memory");
  return s;
}

static inline void nrt_crit_leave(nrt_crit_t s) { __asm volatile("msr primask, %0" : : "r"(s) : "memory"); }

/* The pins of these chips form one bank: the I/O table holds the GPIO number in `mask`. */
#define NRT_GPIO(n) NULL, NULL, NULL, (uint8_t)(n)

#endif /* NRT_PORT_RP_H */
