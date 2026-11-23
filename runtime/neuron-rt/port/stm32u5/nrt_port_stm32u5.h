/* neuron-rt port for the STM32U585 (Arduino UNO Q, the microcontroller side), bare metal,
 * Cortex-M33. The Linux (Qualcomm) side of the UNO Q is a separate host node and is out of
 * scope for this runtime. */
#ifndef NRT_PORT_STM32U5_H
#define NRT_PORT_STM32U5_H

#include <stdint.h>

typedef uint32_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) {
  uint32_t s;
  __asm volatile("mrs %0, primask\n\tcpsid i" : "=r"(s) : : "memory");
  return s;
}

static inline void nrt_crit_leave(nrt_crit_t s) { __asm volatile("msr primask, %0" : : "r"(s) : "memory"); }

/* Port letters as numbers: the I/O table keeps `port * 16 + bit` in `mask`, like the F4 port. */
enum { NRT_PORTA = 0, NRT_PORTB, NRT_PORTC, NRT_PORTD, NRT_PORTE, NRT_PORTF, NRT_PORTG, NRT_PORTH };
#define NRT_PIN(port, ddr, pin, bit) NULL, NULL, NULL, (uint8_t)(NRT_##port * 16u + (bit))

#endif /* NRT_PORT_STM32U5_H */
