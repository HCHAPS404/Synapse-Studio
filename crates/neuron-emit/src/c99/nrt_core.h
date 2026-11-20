/* nrt_core.h: runtime of the C99 code written by `neuron emit c`.
 *
 * Fault protocol. The Neuron reference interpreter stops the program with a fault
 * (Overflow, DivideByZero, ContractViolation) when integer arithmetic leaves the range
 * of its type, when a division has a zero divisor, or when a `requires` clause is false.
 * The generated C reports the same faults through the global `nrt_fault`:
 *
 *   - the first fault of a call chain is recorded, later ones do not replace it;
 *   - the function that detects a fault yields 0 for that operation and the code
 *     continues, so every loop still ends (they are bounded) and no memory is touched;
 *   - a failed `requires` returns 0 at once;
 *   - the caller sets `nrt_fault = 0` before a call and tests it after the call. When it
 *     is not 0 the returned value is meaningless.
 *
 * `nrt_fault` is one global: the generated code is not thread safe. Standard C99 only; the
 * math functions need `-lm` on most systems.
 */
#ifndef NRT_CORE_H
#define NRT_CORE_H

#include <math.h>
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(__GNUC__)
#define NRT_MAYBE_UNUSED __attribute__((unused))
#else
#define NRT_MAYBE_UNUSED
#endif

#define NRT_OK 0
#define NRT_OVERFLOW 1
#define NRT_DIVIDE_BY_ZERO 2
#define NRT_CONTRACT_VIOLATION 3

/* Iteration cap of a `loop` or `while` without `max` (the interpreter's cap). */
#define NRT_LOOP_CAP 1000000u

/* Defined in nrt_core.c: 0, or the code of the first fault since it was cleared. */
extern int nrt_fault;

static inline void nrt_raise(int code) {
    if (nrt_fault == 0) {
        nrt_fault = code;
    }
}

static inline const char *nrt_fault_name(int code) {
    switch (code) {
    case NRT_OK:
        return "";
    case NRT_OVERFLOW:
        return "Overflow";
    case NRT_DIVIDE_BY_ZERO:
        return "DivideByZero";
    case NRT_CONTRACT_VIOLATION:
        return "ContractViolation";
    default:
        return "Unknown";
    }
}

/* ---- signed family: every signed type up to 64 bits, with its range [lo, hi] ---- */

static inline int64_t nrt_in_s(int64_t v, int64_t lo, int64_t hi) {
    if (v < lo || v > hi) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return v;
}

static inline int64_t nrt_add_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if ((b > 0 && a > INT64_MAX - b) || (b < 0 && a < INT64_MIN - b)) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s(a + b, lo, hi);
}

static inline int64_t nrt_sub_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if ((b < 0 && a > INT64_MAX + b) || (b > 0 && a < INT64_MIN + b)) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s(a - b, lo, hi);
}

static inline bool nrt_mul_overflows_s(int64_t a, int64_t b) {
    if (a > 0) {
        if (b > 0) {
            return a > INT64_MAX / b;
        }
        return b < INT64_MIN / a;
    }
    if (b > 0) {
        return a < INT64_MIN / b;
    }
    return a != 0 && b < INT64_MAX / a;
}

static inline int64_t nrt_mul_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if (nrt_mul_overflows_s(a, b)) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s(a * b, lo, hi);
}

static inline int64_t nrt_div_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if (b == 0) {
        nrt_raise(NRT_DIVIDE_BY_ZERO);
        return 0;
    }
    if (a == INT64_MIN && b == -1) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s(a / b, lo, hi);
}

static inline int64_t nrt_rem_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if (b == 0) {
        nrt_raise(NRT_DIVIDE_BY_ZERO);
        return 0;
    }
    if (b == -1) {
        return nrt_in_s(0, lo, hi);
    }
    return nrt_in_s(a % b, lo, hi);
}

static inline int64_t nrt_neg_s(int64_t a, int64_t lo, int64_t hi) {
    if (a == INT64_MIN) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s(-a, lo, hi);
}

static inline int64_t nrt_abs_s(int64_t a, int64_t lo, int64_t hi) {
    if (a == INT64_MIN) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s(a < 0 ? -a : a, lo, hi);
}

/* `a ** e`: the interpreter computes the power exactly and then checks the range. */
static inline int64_t nrt_pow_s(int64_t a, uint32_t e, int64_t lo, int64_t hi) {
    int64_t r = 1;
    uint32_t i;
    if (e == 0) {
        return nrt_in_s(1, lo, hi);
    }
    if (a == 0 || a == 1) {
        return nrt_in_s(a, lo, hi);
    }
    if (a == -1) {
        return nrt_in_s((e % 2 == 0) ? 1 : -1, lo, hi);
    }
    if (e > 63) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    for (i = 0; i < e; i++) {
        r = nrt_mul_s(r, a, INT64_MIN, INT64_MAX);
    }
    return nrt_in_s(r, lo, hi);
}

static inline int64_t nrt_shl_s(int64_t a, uint32_t s, int64_t lo, int64_t hi) {
    if (s >= 128) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    if (s == 63) {
        if (a == -1) {
            return nrt_in_s(INT64_MIN, lo, hi);
        }
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    if (s > 63) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_mul_s(a, (int64_t)1 << s, lo, hi);
}

static inline int64_t nrt_shr_s(int64_t a, uint32_t s, int64_t lo, int64_t hi) {
    if (s >= 128) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    if (s >= 64) {
        return nrt_in_s(a < 0 ? -1 : 0, lo, hi);
    }
    if (a < 0) {
        return nrt_in_s(~((~a) >> s), lo, hi);
    }
    return nrt_in_s(a >> s, lo, hi);
}

/* Wrapping operators: the low `bits` bits of `raw` read as a signed number. */
static inline int64_t nrt_wrap_s(uint64_t raw, int bits) {
    if (bits >= 64) {
        return raw < ((uint64_t)1 << 63) ? (int64_t)raw : -(int64_t)(~raw) - 1;
    } else {
        uint64_t m = raw & (((uint64_t)1 << bits) - 1);
        if (m & ((uint64_t)1 << (bits - 1))) {
            return (int64_t)m - (int64_t)((uint64_t)1 << bits);
        }
        return (int64_t)m;
    }
}

/* Saturating operators. */
static inline int64_t nrt_clamp_range_s(int64_t v, int64_t lo, int64_t hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}

static inline int64_t nrt_sat_add_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if (b > 0 && a > INT64_MAX - b) {
        return nrt_clamp_range_s(INT64_MAX, lo, hi);
    }
    if (b < 0 && a < INT64_MIN - b) {
        return nrt_clamp_range_s(INT64_MIN, lo, hi);
    }
    return nrt_clamp_range_s(a + b, lo, hi);
}

static inline int64_t nrt_sat_sub_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if (b < 0 && a > INT64_MAX + b) {
        return nrt_clamp_range_s(INT64_MAX, lo, hi);
    }
    if (b > 0 && a < INT64_MIN + b) {
        return nrt_clamp_range_s(INT64_MIN, lo, hi);
    }
    return nrt_clamp_range_s(a - b, lo, hi);
}

static inline int64_t nrt_sat_mul_s(int64_t a, int64_t b, int64_t lo, int64_t hi) {
    if (nrt_mul_overflows_s(a, b)) {
        return nrt_clamp_range_s(((a < 0) == (b < 0)) ? INT64_MAX : INT64_MIN, lo, hi);
    }
    return nrt_clamp_range_s(a * b, lo, hi);
}

/* ---- unsigned family: every unsigned type up to 64 bits, with its maximum hi ---- */

static inline uint64_t nrt_in_u(uint64_t v, uint64_t hi) {
    if (v > hi) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return v;
}

static inline uint64_t nrt_add_u(uint64_t a, uint64_t b, uint64_t hi) {
    uint64_t r = a + b;
    if (r < a) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_u(r, hi);
}

static inline uint64_t nrt_sub_u(uint64_t a, uint64_t b, uint64_t hi) {
    if (a < b) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_u(a - b, hi);
}

static inline uint64_t nrt_mul_u(uint64_t a, uint64_t b, uint64_t hi) {
    if (a != 0 && b > UINT64_MAX / a) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_u(a * b, hi);
}

static inline uint64_t nrt_div_u(uint64_t a, uint64_t b, uint64_t hi) {
    if (b == 0) {
        nrt_raise(NRT_DIVIDE_BY_ZERO);
        return 0;
    }
    return nrt_in_u(a / b, hi);
}

static inline uint64_t nrt_rem_u(uint64_t a, uint64_t b, uint64_t hi) {
    if (b == 0) {
        nrt_raise(NRT_DIVIDE_BY_ZERO);
        return 0;
    }
    return nrt_in_u(a % b, hi);
}

/* The interpreter negates exactly: only 0 stays in range of an unsigned type. */
static inline uint64_t nrt_neg_u(uint64_t a) {
    if (a != 0) {
        nrt_raise(NRT_OVERFLOW);
    }
    return 0;
}

static inline uint64_t nrt_pow_u(uint64_t a, uint32_t e, uint64_t hi) {
    uint64_t r = 1;
    uint32_t i;
    if (e == 0) {
        return nrt_in_u(1, hi);
    }
    if (a == 0 || a == 1) {
        return nrt_in_u(a, hi);
    }
    if (e > 63) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    for (i = 0; i < e; i++) {
        r = nrt_mul_u(r, a, UINT64_MAX);
    }
    return nrt_in_u(r, hi);
}

static inline uint64_t nrt_shl_u(uint64_t a, uint32_t s, uint64_t hi) {
    uint64_t r;
    if (s >= 128) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    if (s >= 64) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    r = a << s;
    if ((r >> s) != a) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_u(r, hi);
}

static inline uint64_t nrt_shr_u(uint64_t a, uint32_t s, uint64_t hi) {
    if (s >= 128) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    if (s >= 64) {
        return 0;
    }
    return nrt_in_u(a >> s, hi);
}

static inline uint64_t nrt_sat_add_u(uint64_t a, uint64_t b, uint64_t hi) {
    uint64_t r = a + b;
    if (r < a) {
        r = UINT64_MAX;
    }
    return r > hi ? hi : r;
}

static inline uint64_t nrt_sat_sub_u(uint64_t a, uint64_t b, uint64_t hi) {
    uint64_t r = a < b ? 0 : a - b;
    return r > hi ? hi : r;
}

static inline uint64_t nrt_sat_mul_u(uint64_t a, uint64_t b, uint64_t hi) {
    uint64_t r;
    if (a != 0 && b > UINT64_MAX / a) {
        r = UINT64_MAX;
    } else {
        r = a * b;
    }
    return r > hi ? hi : r;
}

/* ---- shift counts and loop bounds ---- */

/* The interpreter reads a shift count or exponent as a `u32` and faults otherwise. */
static inline uint32_t nrt_cnt_s(int64_t b) {
    if (b < 0 || b > (int64_t)UINT32_MAX) {
        nrt_raise(NRT_OVERFLOW);
        return UINT32_MAX;
    }
    return (uint32_t)b;
}

static inline uint32_t nrt_cnt_u(uint64_t b) {
    if (b > (uint64_t)UINT32_MAX) {
        nrt_raise(NRT_OVERFLOW);
        return UINT32_MAX;
    }
    return (uint32_t)b;
}

static inline uint64_t nrt_bound_s(int64_t v) {
    return v < 0 ? 0u : (uint64_t)v;
}

/* ---- casts: `x as T` checks the range of T ---- */

static inline int64_t nrt_s2s(int64_t v, int64_t lo, int64_t hi) {
    return nrt_in_s(v, lo, hi);
}

static inline int64_t nrt_u2s(uint64_t v, int64_t lo, int64_t hi) {
    if (v > (uint64_t)INT64_MAX) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s((int64_t)v, lo, hi);
}

static inline uint64_t nrt_s2u(int64_t v, uint64_t hi) {
    if (v < 0) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_u((uint64_t)v, hi);
}

static inline uint64_t nrt_u2u(uint64_t v, uint64_t hi) {
    return nrt_in_u(v, hi);
}

/* Real to integer: truncates toward zero, NaN is 0, outside the range is a fault. */
static inline int64_t nrt_f2s(double v, int64_t lo, int64_t hi) {
    double t;
    if (v != v) {
        return nrt_in_s(0, lo, hi);
    }
    t = trunc(v);
    if (t < -9223372036854775808.0 || t >= 9223372036854775808.0) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_s((int64_t)t, lo, hi);
}

static inline uint64_t nrt_f2u(double v, uint64_t hi) {
    double t;
    if (v != v) {
        return 0;
    }
    t = trunc(v);
    if (t < 0.0 || t >= 18446744073709551616.0) {
        nrt_raise(NRT_OVERFLOW);
        return 0;
    }
    return nrt_in_u((uint64_t)t, hi);
}

/* ---- min, max, clamp ---- */

static inline int64_t nrt_min_s(int64_t a, int64_t b) { return a <= b ? a : b; }
static inline int64_t nrt_max_s(int64_t a, int64_t b) { return a >= b ? a : b; }
static inline uint64_t nrt_min_u(uint64_t a, uint64_t b) { return a <= b ? a : b; }
static inline uint64_t nrt_max_u(uint64_t a, uint64_t b) { return a >= b ? a : b; }
static inline double nrt_min_f(double a, double b) { return a <= b ? a : b; }
static inline double nrt_max_f(double a, double b) { return a >= b ? a : b; }
static inline int64_t nrt_clamp_s(int64_t v, int64_t lo, int64_t hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}
static inline uint64_t nrt_clamp_u(uint64_t v, uint64_t lo, uint64_t hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}
static inline double nrt_clamp_f(double v, double lo, double hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}

#ifdef __cplusplus
}
#endif

#endif /* NRT_CORE_H */
