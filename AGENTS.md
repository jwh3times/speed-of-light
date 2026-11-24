# AGENTS.md

Guidance for coding agents (and humans) working in this repository. It holds what applies to
**every** change. Area detail lives in `docs/`, linked below. Read the relevant one before
changing that area.

## What this is

Speed of Light is a Windows GPU tuning / fan control / monitoring tool written in Rust, aiming
to replace MSI Afterburner as the community default. Its differentiator is safety and trust:
writes are gated by a driver compatibility matrix, read back and verified, and fan control
fails towards cooling. See `README.md` for the pitch, `docs/PRD.md` for requirements (IDs like `FAN-3` are
referenced in code reviews and issues), and `ROADMAP.md` for what's next. Licensed
AGPL-3.0-only. Don't add dependencies with incompatible licenses (MIT, Apache-2.0, BSD, MPL-2.0
and LGPL are fine).

## Commands

```powershell
cargo build --workspace
cargo test --workspace                              # hermetic: simulator only, no GPU needed
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                                     # CI runs `cargo fmt --all --check`

cargo test -p sol-nvidia -- --ignored --nocapture   # live READ-ONLY probe, needs an NVIDIA GPU
cargo run -p sol-cli -- probe                       # read-only, real hardware
cargo run -p sol-service -- run --simulate 1        # service on simulated GPUs
cargo run -p sol-cli -- status                      # talk to it
```

In Git Bash, backslashes in `\\.\pipe\...` get mangled. Pass a custom `--pipe` from PowerShell,
or leave the default.

## Layout

- `crates/sol-core`: everything that decides *whether* and *how* to write. No hardware.
  `tuner` (plan / gate / write / verify), `fan` (curve engine), `profile` (TOML),
  `compat` (matrix), `crashguard`, `sim` (simulated GPU with fault injection).
- `crates/sol-nvidia`: NVML backend. `sol-amd` / `sol-intel`: detection stubs.
- `crates/sol-ipc`: protocol types, JSON-lines framing, named pipe with a restrictive DACL.
- `crates/sol-service`: `Service` (sync, transport-free, tested in `tests/service.rs`) plus
  `main.rs` (tokio, pipe, Ctrl+C). Also `backends` (open all vendors) and `conflicts`.
- `crates/sol-cli`: `sol`. Service-backed commands plus direct `probe` / `doctor` / `selftest`.
- `compat/matrix.toml`: compiled into `sol-core`. Entries need evidence.

## Rules for every change

1. **All hardware writes go through `sol_core::tuner`** (`execute` or `reset`). The service's
   fan loop is the one exception: it writes duties computed by `FanController` and clamped by
   `FanCaps`. `sol selftest` uses `tuner::write_and_verify` directly because it *produces* the
   matrix evidence, and it must always restore. Don't add another write path.
2. **Backends are thin.** They translate calls and errors. No range checks, policy or retries
   in `sol-nvidia` and friends: those belong in core, where the simulator can test them.
3. **Missing is `None`, never zero.** A sensor the driver doesn't expose is `None` in
   `Telemetry` / `Capabilities`.
4. **Fail towards cooling.** Any new fan path must hand back to firmware on sensor loss, write
   failure and shutdown. Any clamp on a duty must only ever raise it. See `docs/safety.md`.
   Each invariant there names the test that guards it. Keep those tests passing, and add one
   for each new invariant.
5. **Tests never write to real hardware.** Live tests are `#[ignore]` and read-only. Anything
   that writes uses `sol_core::sim::SimulatedGpu`. Never run `sol selftest --write`, `sol apply`
   or a real (non-`--simulate`) service against the user's GPU without asking them first,
   especially while other tuning tools are running.
6. **Compat matrix entries need evidence**: a hardware-report issue link or `selftest` output.
   Never add a `verified` entry you didn't observe. The build rejects entries with empty
   evidence.
7. **Wire format is a contract.** `sol-ipc` request/response JSON is used by external clients.
   Changing it means bumping `PROTOCOL_VERSION` and updating `wire_format_is_stable`.
   The profile `schema` follows the same rule (`profile::SCHEMA_VERSION`).
8. `unsafe` only in FFI and Win32 code, each block with a `// SAFETY:` comment.
9. `unwrap` / `expect` are lint warnings (errors in CI) outside tests.

## Area guides

- [Architecture](docs/architecture.md): process model, data flow, why Rust.
- [Safety](docs/safety.md): invariants and the tests that guard them.
- [Testing](docs/testing.md): the test layers and how to add hardware coverage.
- [Compatibility matrix](docs/compat-matrix.md): format, precedence, how entries get added.
- [Vendor APIs](docs/vendor-apis.md): NVML / NVAPI / ADLX / IGCL / PresentMon, what's used
  and what's missing.
- [Landscape](docs/landscape.md): competitors and the positioning this project takes.
