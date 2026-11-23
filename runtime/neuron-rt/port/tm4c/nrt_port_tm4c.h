/* neuron-rt port for the TI TM4C123GH6PM (EK-TM4C123GXL, Tiva C LaunchPad), bare metal,
 * Cortex-M4F. Runs from flash at 0x00000000 on the 16 MHz precision internal oscillator
 * (PIOSC); the on-board ICDI debugger flashes it. */
#ifndef NRT_PORT_TM4C_H
#define NRT_PORT_TM4C_H

#include <stdint.h>

typedef uint32_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) {
  uint32_t s;
  __asm volatile("mrs %0, primask\n\tcpsid i" : "=r"(s) : : "memory");
  return s;
}

static inline void nrt_crit_leave(nrt_crit_t s) { __asm volatile("msr primask, %0" : : "r"(s) : "memory"); }

/* Port letters as numbers: the I/O table keeps `port * 16 + bit` in `mask`, like the STM32
 * ports. TM4C has ports A..F (pins 0..7; PF has bits 0..4). */
enum { NRT_PORTA = 0, NRT_PORTB, NRT_PORTC, NRT_PORTD, NRT_PORTE, NRT_PORTF };
#define NRT_PIN(port, ddr, pin, bit) NULL, NULL, NULL, (uint8_t)(NRT_##port * 16u + (bit))

#endif /* NRT_PORT_TM4C_H */
