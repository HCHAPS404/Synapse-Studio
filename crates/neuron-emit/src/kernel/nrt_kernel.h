/* nrt_kernel.h - state table interpreter of the Neuron micro-kernel (version 1).
 *
 * The kernel is independent of the table: it reads the word array that
 * `neuron emit kernel` writes (`<name>.nst.hex`, see spec/formats). C99, no allocation,
 * no recursion; the table must stay valid while the kernel runs.
 *
 *   nrt_kernel_t k;
 *   int16_t in[NRT_MAX_IO];
 *   if (nrt_init(&k, table, table_len) != NRT_OK) { ... }
 *   for (;;) {                       -- once per scan period of the table
 *       in[0] = ...;                 -- inputs, in the order of the table
 *       nrt_scan(&k, in);
 *       for (i = 0; i < k.n_out; i++)
 *           if (k.chg[i]) write_output(i, k.ov[i]);   -- reported changes
 *   }
 *
 * Values are 16-bit signed integers; booleans are 0 and 1.
 */
#ifndef NRT_KERNEL_H
#define NRT_KERNEL_H

#include <stdint.h>

#define NRT_OK 0
#define NRT_ERR_MAGIC 1
#define NRT_ERR_VERSION 2
#define NRT_ERR_SIZE 3
#define NRT_ERR_CHECKSUM 4
#define NRT_ERR_LIMITS 5

#define NRT_MAX_IO 32
#define NRT_MAX_VARS 32
#define NRT_MAX_STACK 16
#define NRT_MAX_ROUNDS 64
#define NRT_MAX_TABLE 1024

typedef struct {
    const uint16_t *tbl; /* the table */
    uint16_t len;        /* its length in words */
    uint16_t n_in, n_out, n_var, n_st, n_tr, initial, flags;
    uint16_t o_var, o_st, o_tr, o_prog; /* offsets of the sections */
    int16_t state;       /* active state */
    int16_t timer;       /* scans since the state was entered */
    uint8_t started;
    int16_t sp;
    int16_t stk[NRT_MAX_STACK];
    int16_t var[NRT_MAX_VARS];
    int16_t ov[NRT_MAX_IO]; /* output values */
    uint8_t ok[NRT_MAX_IO]; /* the output has been written (or all, in a PLC) */
    int16_t pv[NRT_MAX_IO]; /* last reported values */
    uint8_t pk[NRT_MAX_IO];
    uint8_t chg[NRT_MAX_IO]; /* output i changed in the last scan */
    const int16_t *in;
} nrt_kernel_t;

/* Checks the header and the checksum and prepares `k`. Returns NRT_OK or an error. */
int nrt_init(nrt_kernel_t *k, const uint16_t *tbl, uint16_t len);

/* Runs one scan. `in` holds n_in inputs. Fills k->chg and k->ov. */
void nrt_scan(nrt_kernel_t *k, const int16_t *in);

#endif
