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
command line `--test`, `--trace 3s`, `--realtime 10` and `--ros2` (runs the generated
`<module>_ros2.py`, which needs `rclpy`).
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


def T(_):
    """Always true: lets a pattern bind a name inside a condition (`T(x := value)`)."""
    return True


def deep(v):
    if isinstance(v, list):
        return [deep(x) for x in v]
    if isinstance(v, tuple):
        return tuple(deep(x) for x in v)
    if isinstance(v, Obj):
        return Obj(v.id, v.name, {k: deep(x) for k, x in v.f.items()})
    if isinstance(v, Opt) and v.some:
        return Opt(deep(v.v))
    return v


class Early(Exception):
    """`?` found an error or a missing value: the function returns it."""

    def __init__(self, value):
        Exception.__init__(self, "early return")
        self.value = value


class Opt:
    """`Some(x)` (`some` is true) or `none`."""

    __slots__ = ("v", "some")

    def __init__(self, v=None, some=True):
        self.v = v
        self.some = some

    def is_some(self):
        return self.some

    def is_none(self):
        return not self.some

    def unwrap_or(self, default):
        return self.v if self.some else default

    def __eq__(self, o):
        return isinstance(o, Opt) and self.some == o.some and (not self.some or self.v == o.v)

    __hash__ = None


NO = Opt(None, False)


class Res:
    """`Ok(x)` (`ok` is true) or `Err(x)`."""

    __slots__ = ("v", "ok")

    def __init__(self, v, ok):
        self.v = v
        self.ok = ok

    def is_ok(self):
        return self.ok

    def is_err(self):
        return not self.ok

    def unwrap_or(self, default):
        return self.v if self.ok else default

    def __eq__(self, o):
        return isinstance(o, Res) and self.ok == o.ok and self.v == o.v

    __hash__ = None


class Variant:
    """A value of an enum: `Mode.Auto` or `Shape.Circle(3)`."""

    __slots__ = ("id", "enum", "name", "payload")

    def __init__(self, id, enum, name, payload=()):
        self.id = id
        self.enum = enum
        self.name = name
        self.payload = payload

    def __eq__(self, o):
        return (
            isinstance(o, Variant)
            and self.id == o.id
            and self.name == o.name
            and self.payload == o.payload
        )

    __hash__ = None


class Obj:
    """A value of a struct."""

    __slots__ = ("id", "name", "f")

    def __init__(self, id, name, f):
        self.id = id
        self.name = name
        self.f = f

    def __eq__(self, o):
        return isinstance(o, Obj) and self.name == o.name and self.f == o.f

    __hash__ = None


def try_(v):
    """The `?` operator."""
    if isinstance(v, Res):
        if v.ok:
            return v.v
        raise Early(v)
    if isinstance(v, Opt):
        if v.some:
            return v.v
        raise Early(v)
    return v


def in_range(x, lo, hi, inclusive):
    """A range pattern: numbers compare as reals, anything else does not match."""
    if isinstance(x, bool) or not isinstance(x, (int, float)):
        return False
    return x >= lo and (x < hi or (inclusive and x == hi))


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
    if isinstance(v, Opt):
        return show(v.v) if v.some else "none"
    if isinstance(v, Res):
        return ("Ok(" if v.ok else "Err(") + show(v.v) + ")"
    if isinstance(v, Variant):
        if not v.payload:
            return v.enum + "." + v.name
        return v.enum + "." + v.name + "(" + ", ".join(show(x) for x in v.payload) + ")"
    if isinstance(v, Obj):
        return v.name + "(" + ", ".join(k + ": " + show(x) for k, x in v.f.items()) + ")"
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


# ---- function blocks the interpreter ticks itself ----------------------------------
#
# Timers, counters, edge detectors, set/reset latches and the PID controller. Their
# fields live in the dictionary `f` (`jam.q` is `jam.f["q"]`). The module's `blocks`
# activity ticks every one of them each base tick with the inputs they hold (property
# style, `jam.input = x`); `call` sets the inputs, ticks once and returns the output.


def _num(v):
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        return 0.0
    return float(v)


def _int(v):
    if isinstance(v, bool) or not isinstance(v, int):
        return 0
    return v


def native(kind, **a):
    """A new block of `kind` with the constructor arguments `a`."""
    f = {}
    if kind in ("ton", "tof", "tp"):
        f["preset"] = _num(a.get("preset"))
        for k in ("input", "q", "prev", "running"):
            f[k] = False
        f["et"] = 0.0
    elif kind in ("ctu", "ctd", "ctud"):
        f["preset"] = _int(a.get("preset"))
        for k in ("input", "down", "reset", "load", "q", "qu", "qd", "prev", "prev_down"):
            f[k] = False
        f["cv"] = 0
    elif kind in ("r_trig", "f_trig"):
        for k in ("input", "q", "prev"):
            f[k] = False
    elif kind in ("sr", "rs"):
        for k in ("set", "reset", "q", "input", "prev"):
            f[k] = False
    elif kind == "pid":
        f["kp"] = _num(a.get("kp"))
        f["ki"] = _num(a.get("ki"))
        f["kd"] = _num(a.get("kd"))
        f["out"] = a.get("out", (-INF, INF))
        f["antiwindup"] = True if "antiwindup" not in a else truthy(a["antiwindup"])
        f["integral"] = 0.0
        f["last"] = 0.0
    return Native(kind, f)


class Native:
    def __init__(self, kind, f):
        self.kind = kind
        self.f = f

    def _b(self, k):
        return truthy(self.f.get(k))

    def _r(self, k):
        return _num(self.f.get(k))

    def tick(self, dt):
        f, kind = self.f, self.kind
        if kind == "pid":
            return
        inp = self._b("input")
        prev = self._b("prev")
        if kind == "ton":
            preset = self._r("preset")
            et = min(self._r("et") + dt, preset) if inp else 0.0
            f["et"] = et
            f["q"] = inp and et >= preset - 1e-12
        elif kind == "tof":
            preset = self._r("preset")
            q = self._b("q")
            et = self._r("et")
            if inp:
                q = True
                et = 0.0
            elif q:
                et += dt
                if et >= preset - 1e-12:
                    q = False
                    et = preset
            f["et"] = et
            f["q"] = q
        elif kind == "tp":
            preset = self._r("preset")
            running = self._b("running")
            et = self._r("et")
            q = self._b("q")
            if inp and not prev and not running:
                running = True
                et = 0.0
                q = True
            if running:
                et += dt
                if et >= preset - 1e-12:
                    running = False
                    q = False
            f["running"] = running
            f["et"] = et
            f["q"] = q
        elif kind in ("ctu", "ctd", "ctud"):
            rising = inp and not prev
            cv = _int(f.get("cv"))
            preset = _int(f.get("preset"))
            down = self._b("down")
            prev_down = self._b("prev_down")
            if kind == "ctu":
                if self._b("reset"):
                    cv = 0
                elif rising:
                    cv += 1
                f["q"] = cv >= preset
            elif kind == "ctd":
                if self._b("load"):
                    cv = preset
                elif rising:
                    cv -= 1
                f["q"] = cv <= 0
            else:
                if self._b("reset"):
                    cv = 0
                else:
                    if rising:
                        cv += 1
                    if down and not prev_down:
                        cv -= 1
                f["qu"] = cv >= preset
                f["qd"] = cv <= 0
            f["cv"] = cv
            f["prev_down"] = down
        elif kind == "r_trig":
            f["q"] = inp and not prev
        elif kind == "f_trig":
            f["q"] = (not inp) and prev
        elif kind == "sr":
            f["q"] = self._b("set") or (self._b("q") and not self._b("reset"))
        elif kind == "rs":
            f["q"] = (not self._b("reset")) and (self._b("set") or self._b("q"))
        else:
            return
        f["prev"] = inp

    def call(self, x=None, y=None):
        f, kind = self.f, self.kind
        if kind == "pid":
            return self._pid(_num(x))
        f["input" if kind not in ("sr", "rs") else "set"] = truthy(x)
        if kind in ("sr", "rs") and y is not None:
            f["reset"] = truthy(y)
        self.tick(_dt())
        if kind in ("ctu", "ctd", "ctud"):
            return f.get("cv", 0)
        return f.get("q", False)

    def _pid(self, x):
        dt = _dt()
        lo, hi = self.f.get("out", (-INF, INF))
        lo, hi = _num(lo) if lo is not None else -INF, _num(hi) if hi is not None else INF
        kp, ki, kd = self._r("kp"), self._r("ki"), self._r("kd")
        integral, last = self._r("integral"), self._r("last")
        derivative = kd * (x - last) / dt if dt > 0.0 else 0.0
        raw = kp * x + integral + derivative
        out = lo if raw < lo else hi if raw > hi else raw
        saturated = abs(raw - out) > sys.float_info.epsilon
        if not (self._b("antiwindup") and saturated):
            self.f["integral"] = integral + ki * x * dt
        self.f["last"] = x
        return out

    def reset(self):
        f = self.f
        for k in ("integral", "last", "y", "x", "et", "value"):
            if k in f:
                f[k] = 0.0
        for k in ("q", "started", "stable", "running"):
            if k in f:
                f[k] = False
        if "cv" in f:
            f["cv"] = 0


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


class SeqReturn(Exception):
    """`return` in a sequence: the sequence ends."""


class SeqBreak(Exception):
    """`break` in a flattened sequence loop."""


class SeqContinue(Exception):
    """`continue` in a flattened sequence loop."""


class Runner:
    """Position of a running sequence (or one branch of a `parallel` block).

    `ops` are tuples: ("step", name), ("stmt", fn), ("until", cond, timeout, target),
    ("wait", duration), ("jump", name), ("lbegin", kind, a, b, exit), ("lnext",),
    ("lend", head), ("par", [ops, ...]).
    """

    def __init__(self, name, ops):
        self.name = name
        self.ops = ops
        self.pc = 0
        self.wait = None
        self.par = None
        self.done = False
        self.loops = []


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
        self.listener = None  # called as listener(name, value) when a signal changes
        self.L = {}  # locals of sequences, which live across ticks
        self.fsm_info = {}

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

    def blocks(self, items):
        """The module's `blocks` activity: ticks every native block each base tick."""

        def run():
            dt = self.dt()
            for b in items:
                b.tick(dt)

        self._add("blocks", "blocks", 0, self.BASE_TICK_NS, 0, 0, run)

    def fsm(self, name, states, period=None, offset=0, key=None):
        """A state machine. `states`: (name, initial, entry, exit, during, transitions)
        with `transitions`: (target, when, after, action); every part but the name and
        the flag is a function or None. `after` returns nanoseconds."""
        st = {"state": None, "entered": 0}
        self.fsm_info.setdefault(name, (st, states))

        def run():
            cur = st["state"]
            if cur is None:
                cur = next((i for i, s in enumerate(states) if s[1]), 0)
                st["state"] = cur
                st["entered"] = self.now
                if cur < len(states):
                    self._trace("STATE", name, states[cur][0])
                    if states[cur][2]:
                        states[cur][2]()
            if cur >= len(states):
                return
            s = states[cur]
            if s[4]:
                s[4]()
            for target, when, after, action in s[5]:
                fire = when is None and after is None
                if when is not None:
                    fire = truthy(when())
                if not fire and after is not None:
                    fire = self.now - st["entered"] >= after()
                if not fire:
                    continue
                if s[3]:
                    s[3]()
                if action:
                    action()
                ti = next((i for i, x in enumerate(states) if x[0] == target), None)
                if ti is None:
                    break
                st["state"] = ti
                st["entered"] = self.now
                self._trace("STATE", name, states[ti][0])
                if states[ti][2]:
                    states[ti][2]()
                break

        if key is not None:
            self._add(name, "fsm", 1, None, 0, 0, run, key)
        else:
            self._add(name, "fsm", 1, self.DEFAULT_RATE_NS if period is None else period,
                      0, offset, run)

    def state_of(self, name):
        """Name of the current state of the state machine `name` (empty before it runs)."""
        info = self.fsm_info.get(name)
        if info is None or info[0]["state"] is None or info[0]["state"] >= len(info[1]):
            return ""
        return info[1][info[0]["state"]][0]

    def sequence(self, name, ops):
        """A sequence. `ops` is the flattened step list (see `Runner`)."""
        rn = Runner(name, ops)

        def run():
            try:
                self._advance(rn)
            except (SeqBreak, SeqContinue):
                pass  # a break outside any loop ends the tick

        self._add(name, "sequence", 1, self.BASE_TICK_NS, 0, 0, run)

    def _jump(self, rn, target):
        for i, op in enumerate(rn.ops):
            if op[0] == "step" and op[1] == target:
                rn.pc = i
                break
        else:
            rn.done = True
        rn.loops = []

    def _loop_bound(self, fn):
        return LOOP_CAP if fn is None else bound(fn())

    def _advance(self, rn):
        for _ in range(10_000):
            if rn.done:
                return
            if rn.par is not None:
                every = True
                for s in rn.par:
                    self._advance(s)
                    every = every and s.done
                if every:
                    rn.par = None
                    rn.pc += 1
                    continue
                return
            w = rn.wait
            if w is not None:
                if w[0] == "time":
                    if self.now >= w[1]:
                        rn.wait = None
                        rn.pc += 1
                        continue
                    return
                _, cond, deadline, target = w
                if truthy(cond()):
                    rn.wait = None
                    rn.pc += 1
                    continue
                if deadline is not None and self.now >= deadline:
                    rn.wait = None
                    self._trace("EVENT", rn.name, "timeout")
                    if target is not None:
                        self._jump(rn, target)
                    else:
                        rn.pc += 1
                    continue
                return
            if rn.pc >= len(rn.ops):
                rn.done = True
                return
            op = rn.ops[rn.pc]
            k = op[0]
            if k == "step":
                self._trace("STATE", rn.name, op[1])
                rn.pc += 1
            elif k == "lbegin":
                _, kind, a, b, exit_ = op
                st = {"kind": kind, "a": a, "exit": exit_, "head": rn.pc + 1,
                      "items": [], "idx": 0, "n": 0, "bound": 0}
                if kind == "for":
                    st["items"] = list(b())
                else:
                    st["bound"] = self._loop_bound(b)
                rn.loops.append(st)
                rn.pc += 1
            elif k == "lnext":
                if not rn.loops:
                    rn.pc += 1
                    continue
                st = rn.loops.pop()
                if st["kind"] == "for":
                    go = st["idx"] < len(st["items"])
                    if go:
                        item = st["items"][st["idx"]]
                        st["idx"] += 1
                        st["a"](item)
                elif st["n"] >= st["bound"]:
                    go = False
                elif st["kind"] == "while":
                    go = truthy(st["a"]())
                    if go:
                        st["n"] += 1
                else:
                    go = True
                    st["n"] += 1
                if go:
                    rn.pc += 1
                    rn.loops.append(st)
                else:
                    rn.pc = st["exit"]
            elif k == "lend":
                rn.pc = op[1]
            elif k == "par":
                rn.par = [Runner(rn.name, ops) for ops in op[1]]
            elif k == "until":
                _, cond, timeout, target = op
                deadline = None if timeout is None else self.now + timeout()
                rn.wait = ("until", cond, deadline, target)
            elif k == "wait":
                rn.wait = ("time", self.now + op[1]())
            elif k == "jump":
                self._jump(rn, op[1])
            else:  # "stmt"
                try:
                    op[1]()
                except (SeqReturn, Early):
                    rn.done = True
                    return
                except SeqBreak:
                    if not rn.loops:
                        raise
                    rn.pc = rn.loops.pop()["exit"]
                    continue
                except SeqContinue:
                    if not rn.loops:
                        raise
                    rn.pc = rn.loops[-1]["head"]
                    continue
                rn.pc += 1

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
            if self.listener is not None:
                self.listener(n, v)

    def _raise(self, key):
        for a in list(self.subs.get(key, ())):
            self._make_ready(a)

    def emit_event(self, key):
        self._trace("EVENT", key, "")
        self._raise(key)

    def external_event(self, key):
        """An event from outside the program (a ROS 2 message): raised, then handled now."""
        self.emit_event(key)
        self._drain()

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
        except Early:
            pass  # `?` in a task: the activation ends
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
            except (Fault, Early):
                pass
        for f, body in self.on_faults:
            if f is None or f == name:
                try:
                    body()
                except (Fault, Early):
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


def early(f):
    """Marks the body of a function with `ensures`: `?` ends the body with its value."""

    def wrapper(*args):
        try:
            return f(*args)
        except Early as e:
            return e.value

    wrapper.__name__ = f.__name__
    return wrapper


def fn(f):
    """Marks a Neuron function: calls nest at most 100 deep (`fault StackOverflow`)."""

    def wrapper(*args):
        global _depth
        if _depth >= 100:
            raise Fault("StackOverflow")
        _depth += 1
        try:
            return f(*args)
        except Early as e:
            return e.value
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
            try:
                body()
            except Early:
                pass  # `?` in a test body ends the test
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
            import importlib
            import os

            base = os.path.splitext(os.path.basename(mod.__file__))[0]
            try:
                wrapper = importlib.import_module(base + "_ros2")
            except ImportError as e:
                sys.stderr.write("cannot load the ROS 2 wrapper %s_ros2.py: %s\n" % (base, e))
                return 2
            return wrapper.main()
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
