"""Runtime of the Python programs written by `neuron emit python`.

It runs the activities of a `host` module (periodic and event tasks, `when` rules, the
safety logic) on a discrete event queue in simulated time, with the same rules as the
Neuron reference interpreter (spec 14 and 17):

* time is an integer number of nanoseconds and only moves when an event is processed;
* a task reads a snapshot of the signals taken at its release and its writes become
  visible at `release + deadline` (the period unless the task says otherwise): logical
  execution time;
* at one instant events are processed by kind (commit, input, event, release) and then
  the ready activities run by priority and release order;
* a fault stops every activity except the safety logic until `reset when` holds.

Standard library only, Python 3.10 or newer. `main` gives every generated module the
command line `--test`, `--trace 3s`, `--realtime 10` and `--ros2` (not supported).
"""

import heapq
import math
import struct
import sys
import time

INF = float("inf")
NAN = float("nan")
MAX_ARRAY = 1 << 20
LOOP_CAP = 1_000_000
I64_MAX = (1 << 63) - 1


class Fault(Exception):
    """A Neuron `fault`: the program enters its safe state."""

    def __init__(self, name):
        Exception.__init__(self, name)
        self.name = name


class SetupError(Exception):
    """The program cannot start (for example a period that is not positive)."""


# ---- numbers -------------------------------------------------------------------


def f32(x):
    """Rounds to the nearest single precision number, like Rust `x as f32`."""
    try:
        return struct.unpack("f", struct.pack("f", x))[0]
    except OverflowError:
        return INF if x > 0 else -INF


def rnd(x):
    """Rust `f64::round() as i64`: halves away from zero, saturating, NaN is zero."""
    if x != x:
        return 0
    if x >= 9.3e18:
        return I64_MAX
    if x <= -9.3e18:
        return -I64_MAX - 1
    neg = x < 0
    a = -x if neg else x
    f = math.floor(a)
    r = f + 1 if a - f >= 0.5 else f
    return -int(r) if neg else int(r)


def ns(seconds):
    """Seconds as nanoseconds."""
    return rnd(seconds * 1.0e9)


def chk(v, lo, hi):
    """Checked integer arithmetic: `fault Overflow` outside the range of the type."""
    if v < lo or v > hi:
        raise Fault("Overflow")
    return v


def wrap(v, lo, hi):
    return (v - lo) % (hi - lo + 1) + lo


def sat(v, lo, hi):
    return lo if v < lo else hi if v > hi else v


def idiv(a, b):
    if b == 0:
        raise Fault("DivideByZero")
    q = abs(a) // abs(b)
    return q if (a < 0) == (b < 0) else -q


def irem(a, b):
    if b == 0:
        raise Fault("DivideByZero")
    r = abs(a) % abs(b)
    return -r if a < 0 else r


def ipow(a, b):
    if b < 0 or (b > 256 and abs(a) > 1):
        raise Fault("Overflow")
    r = a**b
    if abs(r) >= (1 << 127):
        raise Fault("Overflow")
    return r


def shl(a, b):
    if b < 0 or b >= 128:
        raise Fault("Overflow")
    return a << b


def shr(a, b):
    if b < 0 or b >= 128:
        raise Fault("Overflow")
    return a >> b


def fdiv(a, b):
    try:
        return a / b
    except ZeroDivisionError:
        if a != a or a == 0:
            return NAN
        neg = (a < 0) != (math.copysign(1.0, b) < 0)
        return -INF if neg else INF


def fmod(a, b):
    try:
        return math.fmod(a, b)
    except ValueError:
        return NAN


def fpow(a, b):
    try:
        return math.pow(a, b)
    except OverflowError:
        return INF
    except ValueError:
        return INF if a == 0 else NAN


def _m1(f, x):
    try:
        return f(x)
    except ValueError:
        return NAN
    except OverflowError:
        return INF


def sin(x):
    return _m1(math.sin, x)


def cos(x):
    return _m1(math.cos, x)


def tan(x):
    return _m1(math.tan, x)


def asin(x):
    return _m1(math.asin, x)


def acos(x):
    return _m1(math.acos, x)


def atan(x):
    return math.atan(x)


def atan2(y, x):
    return math.atan2(y, x)


def sqrt(x):
    return math.sqrt(x) if x >= 0 else NAN


def exp(x):
    return _m1(math.exp, x)


def log(x):
    if x == 0:
        return -INF
    return _m1(math.log, x)


def log10(x):
    if x == 0:
        return -INF
    return _m1(math.log10, x)


def hypot(x, y):
    return math.hypot(x, y)


def sign(x):
    return 1 if x > 0 else -1 if x < 0 else 0


def vmin(a, b):
    return a if a <= b else b


def vmax(a, b):
    return a if a >= b else b


def clamp(v, lo, hi):
    return lo if v < lo else hi if v > hi else v


def _whole(x):
    return isinstance(x, int) or x != x or x in (INF, -INF)


def floor(x):
    return x if _whole(x) else float(math.floor(x))


def ceil(x):
    return x if _whole(x) else float(math.ceil(x))


def trunc(x):
    return x if _whole(x) else float(math.trunc(x))


def roundf(x):
    if _whole(x) or abs(x) >= 4503599627370496.0:
        return x
    return float(rnd(x))


def cast_int(v, lo, hi):
    """`x as u8`: truncates toward zero, then checks the range."""
    if isinstance(v, bool):
        v = int(v)
    if isinstance(v, float):
        if v != v:
            v = 0
        elif v == INF:
            v = 1 << 127
        elif v == -INF:
            v = -(1 << 127)
        else:
            v = int(v)
    return chk(v, lo, hi)


def truthy(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, int):
        return v != 0
    return False


# ---- values --------------------------------------------------------------------


def deep(v):
    if isinstance(v, list):
        return [deep(x) for x in v]
    if isinstance(v, tuple):
        return tuple(deep(x) for x in v)
    return v


def at(a, i):
    if isinstance(i, int) and 0 <= i < len(a):
        return a[i]
    raise Fault("ContractViolation")


def setat(a, i, v):
    if isinstance(i, int) and 0 <= i < len(a):
        a[i] = v
        return
    raise Fault("ContractViolation")


def rng(lo, hi, inclusive):
    end = hi + 1 if inclusive else hi
    if end - lo > MAX_ARRAY:
        raise Fault("LoopBound")
    return range(lo, end)


def bound(n):
    return max(0, min(int(n), LOOP_CAP))


def repeat(v, n):
    if n < 0:
        n = 0
    if n > MAX_ARRAY:
        raise Fault("ContractViolation")
    return [deep(v) for _ in range(n)]


def fmt_f64(x):
    """At most six decimals and no trailing zeros, like the interpreter."""
    if x != x:
        return "nan"
    if x == INF:
        return "inf"
    if x == -INF:
        return "-inf"
    s = ("%.6f" % x).rstrip("0").rstrip(".")
    return "0" if s in ("", "-0") else s


def show(v, unit=None):
    """Text of a value in a trace. `unit` is `(text, scale, offset)` for a quantity."""
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        if unit is not None:
            return fmt_f64((v - unit[2]) / unit[1]) + " " + unit[0]
        return fmt_f64(v)
    if isinstance(v, str):
        return v
    if isinstance(v, list):
        return "[" + ", ".join(show(x) for x in v) + "]"
    if isinstance(v, tuple):
        return "(" + ", ".join(show(x) for x in v) + ")"
    return "()"


def fmt_time(t_ns):
    return "%.3f ms" % (t_ns / 1.0e6)


def render(e):
    """One trace line: `   500.000 ms  SIG    messages = 1`."""
    t, tag, name, value = e
    head = "%14s  %-5s  %s" % (fmt_time(t), tag, name)
    return head + " = " + value if value != "" else head


# ---- function blocks (the part of the standard library hosts use) -----------------


def _dt():
    return Runtime.current.dt()


class MovingAverage:
    def __init__(self, window=4):
        self.window = max(1, int(window))
        self.buf = []

    def step(self, x):
        self.buf.append(float(x))
        while len(self.buf) > self.window:
            self.buf.pop(0)
        s = 0.0
        for v in self.buf:
            s += v
        return s / len(self.buf)


class Hysteresis:
    def __init__(self, low=0.0, high=0.0):
        self.low = low
        self.high = high
        self.q = False

    def step(self, x):
        if x >= self.high:
            self.q = True
        elif x <= self.low:
            self.q = False
        return self.q


class Debounce:
    def __init__(self, time=0.0):
        self.time = time
        self.stable = False
        self.since = -1.0

    def step(self, x):
        raw = truthy(x)
        now_s = Runtime.current.now / 1.0e9
        if raw == self.stable:
            self.since = -1.0
        elif self.since < 0.0:
            self.since = now_s
        elif now_s - self.since >= self.time - 1e-12:
            self.stable = raw
            self.since = -1.0
        return self.stable


class Ramp:
    def __init__(self, rate=0.0):
        self.rate = rate
        self.value = 0.0

    def step(self, x):
        step = abs(self.rate) * _dt()
        self.value = self.value + clamp(x - self.value, -step, step)
        return self.value


class LowPass:
    def __init__(self, cutoff=0.0):
        self.cutoff = cutoff
        self.y = 0.0
        self.x = 0.0
        self.started = False

    def _alpha(self):
        dt = _dt()
        rc = 1.0 / (2.0 * math.pi * self.cutoff) if self.cutoff > 0.0 else 0.0
        return dt / (rc + dt) if rc + dt > 0.0 else 1.0

    def step(self, x):
        a = self._alpha()
        ny = x if not self.started else self.y + a * (x - self.y)
        self.y, self.x, self.started = ny, x, True
        return ny


class HighPass(LowPass):
    def step(self, x):
        a = self._alpha()
        ny = 0.0 if not self.started else (1.0 - a) * (self.y + x - self.x)
        self.y, self.x, self.started = ny, x, True
        return ny


# ---- the event queue -------------------------------------------------------------

COMMIT, INPUT, RAISE, RELEASE = 0, 1, 2, 3


class Act:
    def __init__(self, name, kind, prio, period, deadline, fn):
        self.name = name
        self.kind = kind
        self.prio = prio
        self.period = period
        self.deadline = deadline
        self.fn = fn


class RuleState:
    def __init__(self):
        self.prev = False
        self.since = None
        self.fired = False


class Runtime:
    """One running program."""

    current = None
    BASE_TICK_NS = 1_000_000
    DEFAULT_RATE_NS = 10_000_000
    MAX_EVENTS = 5_000_000

    def __init__(self):
        Runtime.current = self
        self.now = 0
        self.queue = []
        self.seq = 0
        self.ready = []
        self.sigs = {}
        self.sig_info = {}
        self.inputs = {}
        self.acts = []
        self.subs = {}
        self.trace = []
        self.sink = None
        self.fault = None
        self.halted = False
        self.safe_states = []
        self.on_faults = []
        self.snap = None
        self.writes = None
        self.period = 0
        self.deadline = 0
        self.rel = 0
        self.in_test = False
        self.fail = None
        self.processed = 0

    # -- setup --

    def signal(self, name, init, unit=None):
        self.sigs[name] = init
        self.sig_info[name] = (name, unit)

    def _add(self, name, kind, prio, period, deadline, offset, fn, key=None):
        a = Act(name, kind, prio, period, deadline, fn)
        self.acts.append(a)
        if period is not None:
            self._push(offset, RELEASE, ("release", a))
        if key is not None:
            self.subs.setdefault(key, []).append(a)
        return a

    def task(self, name, fn, period, offset=0, prio=1, deadline=None):
        if period <= 0:
            raise SetupError("a period must be positive")
        self._add(name, "task", prio, period, period if deadline is None else deadline, offset, fn)

    def task_on(self, name, fn, key, prio=1, deadline=None):
        self._add(name, "task", prio, None, 0 if deadline is None else deadline, 0, fn, key)

    def rule(self, cond, hold, body):
        st = RuleState()
        self._add("when", "rule", 1, self.DEFAULT_RATE_NS, 0, 0,
                  lambda: self.rule_step(st, cond, hold, body))

    def safety(self, rate, members):
        """`members`: ("rule", cond, hold, body), ("reset", cond), ("safe", body),
        ("fault", name or None, body), in source order."""
        states = {}
        for k, m in enumerate(members):
            if m[0] == "rule":
                states[k] = RuleState()
            elif m[0] == "safe":
                self.safe_states.append(m[1])
            elif m[0] == "fault":
                self.on_faults.append((m[1], m[2]))

        def run():
            for k, m in enumerate(members):
                if m[0] == "rule":
                    self.rule_step(states[k], m[1], m[2], m[3])
                elif m[0] == "reset" and self.fault is not None and m[1]():
                    self.fault = None
                    self.halted = False
                    self._trace("RESET", "fault", "")

        self._add("safety", "safety", 2_000_000, max(1, rate), 0, 0, run)

    def rule_step(self, rt, cond, hold, body):
        c = bool(cond())
        if hold is not None:
            d = ns(hold())
            if c:
                if rt.since is None:
                    rt.since = self.now
                if not rt.fired and self.now - rt.since >= d:
                    rt.fired = True
                    body()
            else:
                rt.since = None
                rt.fired = False
        else:
            fire = c and not rt.prev
            rt.prev = c
            if fire:
                body()

    # -- what the program sees --

    def rd(self, name):
        if self.snap is not None:
            return self.snap[name]
        return self.sigs[name]

    def wr(self, name, v):
        unit = self.sig_info[name][1]
        if unit is not None:
            v = float(v)
        if self.snap is not None:
            self.writes.append((name, v))
        else:
            self._commit(name, v)

    def inp(self, name):
        return self.inputs.get(name)

    def dt(self):
        return (self.period if self.period > 0 else self.BASE_TICK_NS) / 1.0e9

    def now_s(self):
        return self.now / 1.0e9

    def elapsed(self):
        return self.now / 1.0e9

    def current_fault(self):
        return self.fault or ""

    def check(self, cond, msg):
        if cond:
            return
        if self.in_test:
            self.fail = msg
            raise Fault("AssertionFailed: " + msg)
        raise Fault("ContractViolation")

    # -- tests --

    def advance(self, seconds):
        self.run_until(self.now + ns(seconds))

    def drive(self, name, v):
        old = self.inputs.get(name)
        self.inputs[name] = v
        was, is_ = truthy(old), truthy(v)
        if was != is_:
            self._raise(name + (".rising" if is_ else ".falling"))
        if old is None or old != v:
            self._raise(name + ".changed")
        self._drain()

    # -- the loop --

    def _push(self, t, rank, ev):
        self.seq += 1
        heapq.heappush(self.queue, (max(t, self.now), rank, self.seq, ev))

    def _trace(self, tag, name, value):
        e = (self.now, tag, name, value)
        self.trace.append(e)
        if self.sink is not None:
            self.sink(render(e))

    def _commit(self, name, v):
        old = self.sigs.get(name)
        changed = name not in self.sigs or old != v
        self.sigs[name] = v
        if changed:
            n, unit = self.sig_info[name]
            self._trace("SIG", n, show(v, unit))

    def _raise(self, key):
        for a in list(self.subs.get(key, ())):
            self._make_ready(a)

    def emit_event(self, key):
        self._trace("EVENT", key, "")
        self._raise(key)

    def _make_ready(self, a):
        if self.halted and a.kind != "safety":
            return
        self.seq += 1
        self.ready.append((a, self.now, self.seq))

    def _process(self, ev):
        k = ev[0]
        if k == "commit":
            self._commit(ev[1], ev[2])
        elif k == "raise":
            self._raise(ev[1])
        elif k == "release":
            a = ev[1]
            if a.period is not None:
                self._push(self.now + a.period, RELEASE, ev)
            self._make_ready(a)

    def _drain(self):
        guard = 0
        while self.ready:
            guard += 1
            if guard > 100_000:
                self.ready = []
                self._trace("EVENT", "event storm", "")
                break
            best = 0
            for k, r in enumerate(self.ready):
                b = self.ready[best]
                if r[0].prio > b[0].prio or (r[0].prio == b[0].prio and r[2] < b[2]):
                    best = k
            a, rel, _ = self.ready.pop(best)
            self._run(a, rel)

    def _run(self, a, rel):
        if self.halted and a.kind != "safety":
            return
        let_sem = a.kind == "task"
        self.snap = dict(self.sigs) if let_sem else None
        self.writes = []
        self.period = a.period or 0
        self.deadline = a.deadline
        self.rel = rel
        err = None
        try:
            a.fn()
        except Fault as f:
            err = f
        writes, deadline = self.writes, self.deadline
        self.snap = None
        self.writes = None
        self.period = 0
        self.deadline = 0
        if err is not None:
            self._raise_fault(err.name)
            return
        for name, v in writes:
            if deadline > 0:
                self._push(rel + deadline, COMMIT, ("commit", name, v))
            else:
                self._commit(name, v)

    def _raise_fault(self, name):
        if self.fault is not None:
            return
        self.fault = name
        self.halted = True
        self._trace("FAULT", name, "")
        self.snap = None
        self.writes = None
        for body in self.safe_states:
            try:
                body()
            except Fault:
                pass
        for f, body in self.on_faults:
            if f is None or f == name:
                try:
                    body()
                except Fault:
                    pass

    def run_until(self, end):
        while self.queue and self.queue[0][0] <= end:
            t = self.queue[0][0]
            self.now = t
            batch = []
            while self.queue and self.queue[0][0] == t:
                batch.append(heapq.heappop(self.queue)[3])
            for ev in batch:
                self.processed += 1
                self._process(ev)
            self._drain()
            if self.processed > self.MAX_EVENTS:
                break
        if self.now < end:
            self.now = end

    def run_realtime(self, seconds):
        """Runs against the wall clock: the same events, each at its instant."""
        start = time.monotonic()
        end = ns(seconds)
        while True:
            t = min(self.queue[0][0], end) if self.queue else end
            delay = start + t / 1.0e9 - time.monotonic()
            if delay > 0:
                time.sleep(delay)
            self.run_until(t)
            if t >= end:
                break


# ---- user functions ----------------------------------------------------------------

_depth = 0


def fn(f):
    """Marks a Neuron function: calls nest at most 100 deep (`fault StackOverflow`)."""

    def wrapper(*args):
        global _depth
        if _depth >= 100:
            raise Fault("StackOverflow")
        _depth += 1
        try:
            return f(*args)
        finally:
            _depth -= 1

    wrapper.__name__ = f.__name__
    return wrapper


# ---- command line --------------------------------------------------------------------

_UNITS = (("ns", 1.0e-9), ("us", 1.0e-6), ("ms", 1.0e-3), ("s", 1.0))


def parse_duration(text):
    """`3s`, `500ms`, `1.5s`; a bare number is seconds. Returns seconds."""
    for suffix, scale in _UNITS:
        if text.endswith(suffix):
            return float(text[: -len(suffix)]) * scale
    return float(text)


def _out(text):
    sys.stdout.write(text + "\n")


def run_tests(mod):
    passed = failed = 0
    for name, body in mod._TESTS:
        mod._boot()
        r = mod.R
        r.in_test = True
        msg = None
        try:
            body()
        except Fault as f:
            if r.fail is not None:
                msg = "assertion failed: " + r.fail if r.fail else "assertion failed"
            else:
                msg = "the program raised fault " + f.name
        except Exception as e:  # a bug in the program or the runtime, never a pass
            msg = "internal error: %s: %s" % (type(e).__name__, e)
        if msg is None:
            passed += 1
            _out("PASS " + name)
        else:
            failed += 1
            _out("FAIL " + name + ": " + msg)
    _out("%d passed, %d failed" % (passed, failed))
    return 1 if failed else 0


def main(mod, argv):
    """Entry point of a generated module."""
    try:
        sys.stdout.reconfigure(newline="\n")
    except (AttributeError, ValueError):
        pass
    args = list(argv)
    try:
        if "--ros2" in args:
            sys.stderr.write("--ros2 is not supported by this emitter\n")
            return 2
        if "--test" in args:
            return run_tests(mod)
        if "--trace" in args:
            end = ns(parse_duration(args[args.index("--trace") + 1]))
            mod._boot()
            mod.R.run_until(end)
            for e in mod.R.trace:
                _out(render(e))
            return 0
        if "--realtime" in args:
            secs = parse_duration(args[args.index("--realtime") + 1])
            mod._boot()
            mod.R.sink = _out
            sys.stdout.flush()
            mod.R.run_realtime(secs)
            return 0
    except (IndexError, ValueError):
        sys.stderr.write("usage: python %s [--test | --trace 3s | --realtime 10s]\n" % mod.__file__)
        return 2
    except SetupError as e:
        sys.stderr.write("cannot start: %s\n" % e)
        return 3
    sys.stderr.write("usage: python %s [--test | --trace 3s | --realtime 10s]\n" % mod.__file__)
    return 2
