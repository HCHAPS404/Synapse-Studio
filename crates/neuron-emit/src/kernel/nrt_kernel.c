/* nrt_kernel.c - state table interpreter of the Neuron micro-kernel (version 1).
 * Table independent; see nrt_kernel.h and the description in
 * crates/neuron-emit/src/kernel/mod.rs. */
#include "nrt_kernel.h"

#define NRT_NONE 0xFFFFu

static void nrt_push(nrt_kernel_t *k, int16_t v)
{
    if (k->sp < NRT_MAX_STACK) {
        k->stk[k->sp] = v;
        k->sp++;
    }
}

static int16_t nrt_pop(nrt_kernel_t *k)
{
    if (k->sp > 0) {
        k->sp--;
        return k->stk[k->sp];
    }
    return 0;
}

/* Runs the block at offset `cp` of the program. Returns the top of its stack. */
static int16_t nrt_run(nrt_kernel_t *k, uint16_t cp)
{
    uint16_t pc;
    uint16_t op;
    uint16_t arg;
    int16_t a;
    int16_t b;
    k->sp = 0;
    if (cp == NRT_NONE) {
        return 0;
    }
    pc = (uint16_t)(k->o_prog + cp);
    while (pc < k->len) {
        op = k->tbl[pc];
        pc++;
        if (op == 0u) {
            break;
        }
        arg = 0u;
        if ((op >= 1u && op <= 4u) || op == 6u || op == 7u) {
            arg = k->tbl[pc];
            pc++;
        }
        switch (op) {
        case 1: nrt_push(k, (int16_t)arg); break;
        case 2: nrt_push(k, arg < k->n_in ? k->in[arg] : (int16_t)0); break;
        case 3: nrt_push(k, k->var[arg]); break;
        case 4: nrt_push(k, k->ov[arg]); break;
        case 5: nrt_push(k, k->timer); break;
        case 6:
            k->ov[arg] = nrt_pop(k);
            k->ok[arg] = 1;
            break;
        case 7: k->var[arg] = nrt_pop(k); break;
        case 8:
            a = nrt_pop(k);
            nrt_push(k, (int16_t)(a == 0));
            break;
        case 20:
            a = nrt_pop(k);
            nrt_push(k, (int16_t)(-a));
            break;
        case 21: k->timer = 0; break;
        default:
            b = nrt_pop(k);
            a = nrt_pop(k);
            switch (op) {
            case 9: nrt_push(k, (int16_t)(a != 0 && b != 0)); break;
            case 10: nrt_push(k, (int16_t)(a != 0 || b != 0)); break;
            case 11: nrt_push(k, (int16_t)(a == b)); break;
            case 12: nrt_push(k, (int16_t)(a != b)); break;
            case 13: nrt_push(k, (int16_t)(a < b)); break;
            case 14: nrt_push(k, (int16_t)(a <= b)); break;
            case 15: nrt_push(k, (int16_t)(a > b)); break;
            case 16: nrt_push(k, (int16_t)(a >= b)); break;
            case 17: nrt_push(k, (int16_t)(a + b)); break;
            case 18: nrt_push(k, (int16_t)(a - b)); break;
            case 19: nrt_push(k, (int16_t)(a * b)); break;
            default: break;
            }
            break;
        }
    }
    return k->sp > 0 ? k->stk[k->sp - 1] : (int16_t)0;
}

int nrt_init(nrt_kernel_t *k, const uint16_t *tbl, uint16_t len)
{
    uint32_t acc = 0;
    uint16_t i;
    if (len < 12u) {
        return NRT_ERR_SIZE;
    }
    if (tbl[0] != 0x4E53u) {
        return NRT_ERR_MAGIC;
    }
    if (tbl[1] != 1u) {
        return NRT_ERR_VERSION;
    }
    if (tbl[10] != len) {
        return NRT_ERR_SIZE;
    }
    for (i = 0; i < len; i++) {
        if (i != 11u) {
            acc = (acc + (uint32_t)tbl[i] * ((uint32_t)i + 1u)) % 65521u;
        }
    }
    if (tbl[11] != (uint16_t)acc) {
        return NRT_ERR_CHECKSUM;
    }
    if (tbl[3] > NRT_MAX_IO || tbl[4] > NRT_MAX_IO || tbl[5] > NRT_MAX_VARS || len > NRT_MAX_TABLE) {
        return NRT_ERR_LIMITS;
    }
    k->tbl = tbl;
    k->len = len;
    k->flags = tbl[2];
    k->n_in = tbl[3];
    k->n_out = tbl[4];
    k->n_var = tbl[5];
    k->n_st = tbl[6];
    k->n_tr = tbl[7];
    k->initial = tbl[8];
    k->o_var = 12u;
    k->o_st = (uint16_t)(k->o_var + k->n_var);
    k->o_tr = (uint16_t)(k->o_st + 5u * k->n_st);
    k->o_prog = (uint16_t)(k->o_tr + 3u * k->n_tr);
    if ((uint32_t)k->o_prog + (uint32_t)tbl[9] != (uint32_t)len || k->initial >= k->n_st) {
        return NRT_ERR_SIZE;
    }
    k->started = 0;
    k->state = 0;
    k->timer = 0;
    k->sp = 0;
    k->in = 0;
    for (i = 0; i < NRT_MAX_IO; i++) {
        k->ov[i] = 0;
        k->ok[i] = 0;
        k->pv[i] = 0;
        k->pk[i] = 0;
        k->chg[i] = 0;
    }
    for (i = 0; i < NRT_MAX_VARS; i++) {
        k->var[i] = 0;
    }
    return NRT_OK;
}

void nrt_scan(nrt_kernel_t *k, const int16_t *in)
{
    uint16_t i;
    uint16_t rnd;
    uint16_t rounds;
    k->in = in;
    if (!k->started) {
        k->started = 1;
        for (i = 0; i < k->n_var; i++) {
            k->var[i] = (int16_t)k->tbl[k->o_var + i];
        }
        for (i = 0; i < k->n_out; i++) {
            k->ov[i] = 0;
            k->ok[i] = (uint8_t)((k->flags & 2u) != 0u);
            k->pv[i] = 0;
            k->pk[i] = 0;
        }
        k->state = (int16_t)k->initial;
        k->timer = 0;
        (void)nrt_run(k, k->tbl[k->o_st + 5u * (uint16_t)k->state]);
    }
    rounds = (k->flags & 1u) != 0u ? NRT_MAX_ROUNDS : 1u;
    for (rnd = 0; rnd < rounds; rnd++) {
        const uint16_t *s = k->tbl + k->o_st + 5u * (uint16_t)k->state;
        uint16_t first = s[3];
        uint16_t count = s[4];
        uint16_t j;
        int fired = 0;
        (void)nrt_run(k, s[2]);
        for (j = 0; j < count; j++) {
            const uint16_t *t = k->tbl + k->o_tr + 3u * (uint16_t)(first + j);
            int go = 1;
            if (t[1] != NRT_NONE) {
                go = nrt_run(k, t[1]) != 0;
            }
            if (go) {
                (void)nrt_run(k, s[1]);
                (void)nrt_run(k, t[2]);
                k->state = (int16_t)t[0];
                k->timer = 0;
                (void)nrt_run(k, k->tbl[k->o_st + 5u * t[0]]);
                fired = 1;
                break;
            }
        }
        if (!fired) {
            break;
        }
    }
    if (k->timer < 32767) {
        k->timer++;
    }
    for (i = 0; i < k->n_out; i++) {
        k->chg[i] = 0;
        if (k->ok[i]) {
            if (!k->pk[i] || k->pv[i] != k->ov[i]) {
                k->chg[i] = 1;
            }
            k->pv[i] = k->ov[i];
            k->pk[i] = 1;
        }
    }
}
