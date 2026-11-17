/* neuron-rt host port: simulated time and a scenario file, for the differential tests. */
#ifndef NRT_PORT_HOST_H
#define NRT_PORT_HOST_H

#include <stdint.h>

typedef uint8_t nrt_crit_t;

static inline nrt_crit_t nrt_crit_enter(void) { return 0; }
static inline void nrt_crit_leave(nrt_crit_t s) { (void)s; }

/* The host has no registers: the arguments are ignored. */
#define NRT_PIN(port, ddr, pin, bit) NULL, NULL, NULL, (uint8_t)(1u << (bit))

struct nrt_app;
int nrt_main(const struct nrt_app *app, int argc, char **argv);

#endif /* NRT_PORT_HOST_H */
