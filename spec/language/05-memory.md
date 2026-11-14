# 05 · Memory and ownership

## 5.1 Memory model

| Region | Holds | Lifetime |
|---|---|---|
| Static | module `var`, `signal`, `io` state, class instances declared at module level, pools, channels | whole program |
| Read-only | `const` data, string literals, lookup tables | whole program; placed in flash on Harvard targets (AVR) automatically |
| Stack | function locals and temporaries | the call |
| Heap | only in `host` (`list<T>`, `map<K,V>`, `text`) | managed by ownership |

There is **no garbage collector and no implicit heap** in device dialects. Every
byte a device program uses is known at compile time.

> **Property MEM-1 (memory budget).** For every accepted device program, the sum of
> static data, the maximum stack depth of every task and interrupt (with nesting
> allowed by priorities) and the runtime's own needs is less than or equal to the
> RAM declared by the target; otherwise compilation fails with `NE0604` showing
> the breakdown.

## 5.2 Value semantics

All types have **value semantics**: assignment and returning copy the value
(the compiler elides copies where it can). There are no pointers and no null.
Copies larger than 64 bytes in device dialects trigger the lint `NL0501`.

## 5.3 Parameter modes and aliasing

| Mode | Declaration | Call site | Rights |
|---|---|---|---|
| read (default) | `p: T` | `f(x)` | read only; passed by reference or by value at the compiler's choice |
| exclusive | `mut p: T` | `f(mut x)` | read and write; changes are visible to the caller |

Rules (checked statically, `NE0303`):

1. During a call, an argument passed `mut` SHALL NOT be accessible through any other
   argument or through a global that the callee (transitively) reads or writes.
2. References cannot be stored in variables, fields or returned (no lifetimes are
   needed in edition 2026).
3. Slices `[T]` exist only as parameters, viewing an array argument.

These rules give the guarantees of Rust's borrow checker for the subset that
automation code needs, without lifetime annotations.

## 5.4 Pools

`pool<T, N>` provides fixed-capacity allocation of `N` objects of type `T`:

```neuron
var frames: pool<Frame, 4>

fn on_rx(data: [u8]):
  match frames.alloc():
    none => stats.dropped += 1
    h =>
      frames[h].fill(data)
      rx_queue.send(h)
```

- `alloc()` returns a handle `Handle<T>?`; `free(h)` releases it.
- Handles carry a generation counter: using a freed handle is detected and raises
  `fault StaleHandle` (it can never read reused memory).

## 5.5 Channels

`channel<T, N>` is a bounded FIFO, safe between tasks and interrupts:

```neuron
var commands: channel<Command, 8>

on interrupt uart.rx (prio 4):
  if not commands.send(parse(uart.byte())):
    stats.overrun += 1

task executor every 5 ms (prio 2):
  while let cmd = commands.recv() max 8:
    execute(cmd)
```

- `send` returns `false` when full; `recv` returns `T?`. Neither blocks.
- With one producer and one consumer the implementation is lock-free; otherwise
  operations are short critical sections at the resource ceiling.

## 5.6 Shared static state

Module `var`s accessed by more than one task or interrupt are **shared resources**.
The compiler computes, for each, the set of accessing activities and their
priorities (06). No annotation is required; `reads`/`writes` on a task are optional
and, if present, checked.

On targets where an access is not atomic (for example a 16-bit variable on an
8-bit AVR, or any read-modify-write), the compiler protects every access from a
lower priority activity with a critical section at the resource ceiling.

> **Property MEM-2 (no torn reads).** For every shared variable and every
> execution, any read observes a value that some write stored completely.
