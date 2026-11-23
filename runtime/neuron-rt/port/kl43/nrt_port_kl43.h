/* neuron-rt port for the Kinetis MKL43Z256VLH4 (FRDM-KL43Z), bare metal, Cortex-M0+. */
#ifndef NRT_PORT_KL43_H
#define NRT_PORT_KL43_H

#include <stdint.h>

typedef uint32_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) {
  uint32_t s;
  __asm volatile("mrs %0, primask\n\tcpsid i" : "=r"(s) : : "memory");
  return s;
}

static inline void nrt_crit_leave(nrt_crit_t s) { __asm volatile("msr primask, %0" : : "r"(s) : "memory"); }

/* The KL43 has five GPIO ports (A..E), 32 pins each. The I/O table keeps `port * 32 + pin`
 * in `mask` (PTE31 = 4 * 32 + 31 = 159, fits in uint8_t). */
enum { NRT_PORTA = 0, NRT_PORTB = 1, NRT_PORTC = 2, NRT_PORTD = 3, NRT_PORTE = 4 };
#define NRT_PIN(port, ddr, pin, bit) NULL, NULL, NULL, (uint8_t)(NRT_##port * 32u + (bit))

#endif /* NRT_PORT_KL43_H */
