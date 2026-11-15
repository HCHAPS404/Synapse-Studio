# Repository tools

Windows PowerShell scripts for local work. Each one writes its output to `out/` (ignored
by git), because long command output in an IDE terminal can be cut off.

| Script | What it does | Log |
|---|---|---|
| `rust-check.ps1 [-Golden]` | `cargo fmt`, optional refresh of the golden files (parser outlines, NIR of the examples, NIR schema), `clippy -D warnings`, tests | `out/rust-check.log` |
| `clean-check.ps1` | Deletes `target/` and runs `rust-check.ps1` from scratch (after a crash or disk repair) | `out/clean.log`, `out/rust-check.log` |
| `test-retry.ps1` | Workspace tests, retrying executables blocked by Windows Smart App Control (os error 4551) | `out/test-retry.log` |
| `sema-check.ps1 [-Test] [-Filter name]` | Clippy and tests for `neuron-sema` only | `out/sema-check.log` |
| `a9-test.ps1 [-Golden] [-Quick]` | NIR tests (A9) with a short summary; `-Golden` regenerates the golden NIR, `-Quick` skips the proptest | `out/a9.log`, `out/a9-summary.txt` |
| `cli-demo.ps1` | Runs `neuron check` on `examples/` and on a program with deliberate errors | `out/cli.log` |
| `verify-local.ps1` | Everything CI runs: Kiro guard, Rust, Python | `out/verify.log` |
| `verify-worktree.ps1` | Compares every tracked file with its committed content, to detect disk corruption | `out/verify-worktree.log` |
| `restore-from-git.ps1 -Paths a,b [-Force]` | Restores files from git; `-Force` deletes the working copy first | `out/restore.log` |
| `git-publish.ps1 -Message "..." [-Branch develop]` | Commits with DCO sign-off and pushes | `out/git-publish.log` |
| `promote-main.ps1` | Fast-forwards `main` to `develop` | `out/promote-main.log` |
| `check-kiro-not-ignored.ps1` | Fails if `.kiro` is gitignored (also runs in CI) | console |
| `rebuild-venv.ps1` | Recreates the Python virtual environment of `services/core` | `out/venv.log` |
| `probe-toolchains.ps1` | Lists installed MCU toolchains (for task B1) | `out/toolchains.log` |

Diagnostic codes are checked by a test (`crates/neuron-sema/tests/catalogue.rs`), not by a
script: every `NE`/`NL` code used in the compiler must be in
`spec/language/10-diagnostics.md` and documented in `docs/errors/`.
