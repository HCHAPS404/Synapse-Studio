# Machine-readable CLI output (`--format json`)

`neuron check`, `neuron build` and `neuron emit` accept `--format json` (or
`--format=json`). Standard output then holds exactly one JSON document and nothing else;
the human text is not printed. `--format human` is the default and its output is unchanged.
The process exit code is the same in both modes and stays the authority (0 success, 1 the
program or the build has errors, 2 usage error). The Core API compile runner
(`services/core`, task E2.3) uses this mode.

```json
{
  "format": 1,
  "command": "build",
  "ok": false,
  "neuron_version": "0.1.0",
  "language_edition": "2026",
  "diagnostics": [
    {
      "code": "NE0603",
      "severity": "error",
      "message": "`D99` is not a pin of the board `arduino_uno_r3`",
      "file": "b.neuron",
      "line": 2,
      "column": 1,
      "help": "pins with this function: A0, A1, ...",
      "notes": []
    }
  ],
  "messages": [],
  "output_dir": null,
  "artifacts": []
}
```

Keys are printed in alphabetical order; they are grouped here for reading. A successful
build has `"output_dir": "build/arduino_uno_r3"` and `"artifacts": [{"path": "app.hex",
"size": 1234}, ...]`.

| Key | Meaning |
|---|---|
| `format` | Version of this document layout (1). |
| `ok` | `true` when no diagnostic has severity `error`. |
| `diagnostics` | Every `error[...]`, `warning[...]` and `note[...]` of the run, in the order printed. `code`, `file`, `line`, `column` and `help` are `null` when the stage gives none (a missing tool, a usage error). `line` and `column` are 1-based. |
| `messages` | Other text the command printed to standard error (tool output, `built ...`). At most 200 lines. |
| `output_dir` | The directory the command wrote to, or `null` (`check`, or nothing written). |
| `artifacts` | Every regular file under `output_dir`, sorted, with `/` separators and the size in bytes. Hashes are the caller's job. Links are skipped. |

Diagnostics come from the same rendering the human mode prints, so the two modes cannot
disagree; a test compares them. Output is deterministic: the same input and the same
binary give the same bytes.
