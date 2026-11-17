/* neuron-rt port for the ESP32-S3 on ESP-IDF (FreeRTOS underneath). Built with `idf.py`. */
#ifndef NRT_PORT_ESP_IDF_H
#define NRT_PORT_ESP_IDF_H

#include <stdint.h>

typedef uint32_t nrt_crit_t;

nrt_crit_t nrt_crit_enter(void);
void nrt_crit_leave(nrt_crit_t s);

/* Pins are GPIO numbers. */
#define NRT_GPIO(n) NULL, NULL, NULL, (uint8_t)(n)

/* ESP-IDF starts the application at app_main(); the port calls the generated main. */
#define main nrt_generated_main
int nrt_generated_main(void);

#endif /* NRT_PORT_ESP_IDF_H */
