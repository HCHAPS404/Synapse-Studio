# Service errors (Phase E)

Source of truth for the catalogue: `spec/language/10-diagnostics.md`. These codes are
returned by the Core API (`services/core`) and the Edge Daemon as
`{"code": "NE11xx", "message": "...", "request_id": "..."}`; the compiler never emits
them. The design is in `.kiro/specs/core-services/`. `neuron explain NE1101` prints the
matching section. Messages never contain tokens, passwords or connection strings.

### NE1101 job rejected (size or limit)

A compile job was refused. Most often this happens before it runs. Causes: a file path that
is absolute, contains `..` or a backslash, starts with a dot, or is not in normal form;
more files or more bytes than the limits (defaults: 200 files, 200 KiB per file, 2 MiB in
total); two paths that differ only in letter case; an option that does not fit the command
(for example `board` on `check`) or a name that is not a plain name; or a command other than
`check`, `build` or `emit`. In these cases nothing was executed. The same code is used when
the job ran but its output files exceed the caps (200 files, 20 MiB each, 64 MiB in total);
the result is then discarded. Reduce the project or rename the path, then retry.

### NE1102 job timed out or killed

A compile job ran past its wall-clock limit (default 30 s), printed more than 1 MiB on
standard output or error, or was ended by a CPU, memory or file-size limit. The runner kills
the whole process group, so no helper process survives, and deletes the job directory. The
same input on the same `neuron` binary behaves the same way, so shrink the project or split
the build instead of retrying.

### NE1103 rate limit

The client (IP, guest id or user) sent more requests, compile jobs or control messages than
its token bucket allows, or the compile queue is full. The `Retry-After` header gives the
seconds to wait.

### NE1104 guest quota or sandbox expired

A guest sandbox reached a quota (projects, files per version, file size, builds per day) or
its 24-hour life ended. Start a new sandbox with `POST /v1/guest`; the data of an expired
sandbox is deleted.

### NE1105 authentication failed

The login, token or service token is missing, wrong, expired, revoked or tampered with. The
response never says which part failed.

### NE1106 permission denied

The role of the caller does not hold the permission the route needs. A guest gets this code
on every device route. See the permission matrix in `.kiro/specs/core-services/design.md`.

### NE1107 version conflict

An update carried a `row_version` that is no longer the stored one: someone else changed
the resource. Read it again and reapply the change. The same code is returned when two
versions of one project are created at the same moment and this one lost the number; retry.

### NE1108 package signature invalid

A deployment package has no signature, a signature that does not match its manifest, a
file whose SHA-256 differs from the manifest, or a key the daemon does not trust. The
package is kept only in the quarantine folder and is never applied.

### NE1109 command not allowed

A command sent to the Edge Daemon is not in its allowlist, or its arguments fail the
validation of that command. There is no command that runs a shell, edits the allowlist or
touches an E-STOP.

### NE1110 deployment blocked

A deployment step was requested out of order or without its precondition: a passing build
with verification, a dry run, reduced speed and two recorded human confirmations. The state
machine admits no shortcut. The software E-STOP is supplementary; a hardwired E-STOP is the
authority.

### NE1111 device offline or not paired

The target device or its daemon is not connected, is not paired to the cell, or missed its
heartbeat for 15 seconds. Pair it again or check the daemon.

### NE1112 timing profile invalid

A calibration profile has fewer samples than the minimum, a non-positive value, or an unknown
unit. Profiles with `trust = "measured"` need the samples that justify them.
