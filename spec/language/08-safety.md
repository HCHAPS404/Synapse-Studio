# 08 · Safety

Neuron is a design aid, not a certified safety tool (see the repository safety
notice). Within that scope, safety is built into the language in five layers.

| Layer | Mechanism |
|---|---|
| 1. Language | No undefined behavior, no null, no pointers, explicit overflow policy, bounded loops, unit checking |
| 2. Capabilities | Hardware only through `io` or `uses`; exclusive ownership; `trusted` marks unverified code |
| 3. Static analysis | Effects, SRP (no races, no deadlock), stack and memory budget, WCET and deadlines, state-machine verification |
| 4. Physical | Mandatory safe state for actuators, faults, zones, workspace bounds, electrical compatibility, watchdogs |
| 5. Runtime and deployment | Watchdog and memory protection configured, stack guards, image CRC, signed packages, commissioning gates |

## 8.1 The `safety` block

```neuron
safety:
  rate 5 ms
  watchdog 50 ms

  safe_state:
    heater.duty = 0 %
    motor.write(false)
    valve.write(false)

  when temp > 90 degC:
    fault Overheat
  when estop.read() == false:
    fault EmergencyStop

  on fault:
    dbg.print("fault {fault.name}")
```

- `safe_state:` lists the outputs and their safe values. **Every output-capable
  binding** (`gpio.output`, `pwm`, `dac`, motion, PLC outputs, HDL outputs to pins)
  SHALL appear in it, or have a safe value in the board/device manifest; otherwise
  compilation fails with `NE0802`.
- Rules inside `safety` run at the highest task priority at `rate` (default the
  fastest task period, at most 10 ms).
- `watchdog D` enables the hardware watchdog with timeout `D`; the runtime feeds it
  only while every task meets its deadline.
- `on fault:` runs after the safe state has been applied; it MAY log or notify but
  SHALL NOT drive outputs (`NE0803`).

## 8.2 Faults

- `fault Name` (from any task, rule, fsm or sequence) immediately applies the safe
  state, stops all tasks except `safety` and records the fault.
- Built-in faults: `Overflow`, `DivideByZero`, `ContractViolation`, `StaleHandle`,
  `DeadlineMiss`, `StackOverflow`, `Watchdog`, `LinkLost`, `EmergencyStop`.
- Leaving the fault state requires an explicit `reset` from the safety logic or a
  power cycle; the reset condition is declared: `reset when ack.rising and not
  estop_active`.

> **Property SAFE-1.** In every execution, from the instant a fault is raised until
> reset, every output holds its declared safe value.
>
> **Property SAFE-2.** A fault raised in any activity is applied within one safety
> period (or immediately when raised from an interrupt).

## 8.3 Verification properties written by users

```neuron
verify:
  never arm.in_zone(press) and press.cylinder.extended
  always motor.on implies guard.closed
  eventually_within 5 s after start: Running
```

`verify` blocks are checked by the model checker on state machines, sequences and
cells. A violation produces a **counterexample trace** that can be replayed in the
digital twin.

## 8.4 Trusted code and audit

`trusted asm` and `trusted extern` code is excluded from proofs. Every build produces
an **audit report** listing trusted code, assumptions (timing profiles,
manifests with lower trust levels) and disabled checks. A deployment package records
the hash of this report.

## 8.5 Deployment gates

The toolchain (not the language) enforces, for real hardware:

1. compilation without errors and verification passing;
2. the program simulated in the digital twin for the selected scenarios;
3. manifests at trust level *verified* (AI-generated manifests cannot deploy);
4. two explicit human confirmations, then a dry run and reduced-speed commissioning
   for motion devices;
5. signed package checked by the Edge Daemon.

## 8.6 Out of scope

Neuron SHALL NOT generate, download or modify safety-rated logic: fail-safe PLC
programs, safety controller configuration of robots, or safety relay configuration.
It MAY read their status as inputs. The hardwired emergency stop is always the
authority.
