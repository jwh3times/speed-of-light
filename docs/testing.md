# Testing

The bet this project makes is that disciplined testing is how a community tool beats an
established one. One person can't own every combination of GPU × VBIOS × driver × laptop and
desktop, so the strategy is layered. Each layer catches what the one below can't.

## Layer 1: unit and property tests (hermetic, every PR)

`cargo test --workspace`. No GPU, no network, no admin rights.

- `sol-core`: fan curve math and controller (including `proptest` properties), profile parsing,
  the compat matrix, the tuner's plan/gate/verify logic, the crash guard, simulator physics.
- `sol-ipc`: protocol round trips, a pinned wire format, framing (malformed and oversized
  input), and a real named-pipe round trip including the anti-squatting check.
- `sol-nvidia`: error and throttle-reason mapping.
- `sol-service`: the Windows service's registration (account, start type, recovery actions, and
  the stored arguments parsing back into the same settings), which backend states are retried,
  the rule behind `run`'s refusal while the installed service owns the GPUs, and the
  startup-profile trust check
  against real ACLs on temporary files.
- `sol-cli`: what `sol` prints when nothing serves the pipe, by running the real `sol` binary
  against an unserved pipe name.

## Layer 2: closed-loop simulation (hermetic, every PR)

`crates/sol-service/tests/service.rs` runs the real `Service` against `SimulatedGpu`. That
covers the thermal model, firmware auto-fan, the power cap and fault injection (ignored writes,
permission errors, dead sensors, failing fan writes, lost device), a driver that is still
loading when the service starts, a tuning tool that starts later, and uninstall's return to
stock. Simulated time advances in
1 s steps, so a 10-minute load test runs in milliseconds. These tests assert outcomes ("the
GPU never reaches slowdown"), not implementation details.

To add a fault, add a flag to `sim::Faults`, implement it in `SimulatedGpu`, and write a
service test that asserts the safe outcome.

### Service acceptance run (manual, elevated, simulated GPUs)

Some of the Windows service can't run in the hermetic suite: the Service Control Manager's stop
and shutdown requests reaching the graceful stop, the restart after the process is killed, the
owner change on the state folder (it needs elevation), the log files, `run`'s refusal while the
service runs, and `sol`'s "not installed / stopped" message. Those are checked by hand from an
elevated terminal with `sol-service install --simulate 1`, so no real GPU is touched: `sol
status`, kill the process and see Windows restart it (`sol status` answers again, and the log has
a second `service started`), check that `sol-service run` refuses while it runs, stop and start it
from the Services console, check the folder's owner and ACL and the log files,
`sol-service uninstall`, then check `sol`'s "not installed" message. The steps are on the wiki
page [Service-Acceptance-Run-2026-11-26](https://github.com/jwh3times/speed-of-light/wiki/Service-Acceptance-Run-2026-11-26);
the first run passed every step (#35).

## Layer 3: live read-only probes (manual, on real hardware)

`cargo test -p sol-nvidia -- --ignored --nocapture` enumerates devices and reads capabilities,
telemetry and tuning state, and sanity-checks the values. It never writes. Run it on every GPU
you can get access to, on every new driver.

## Layer 4: hardware selftest (manual, opt-in, writes)

`sol selftest --write` (elevated, other tuning tools closed) nudges each control: power limit
−10 %, core +15 MHz, memory +50 MHz, fans to 60 %. It reads each one back, restores the
original values, and confirms the restore. It prints ready-to-paste `compat/matrix.toml` entries.
This is how the matrix grows. See [compat-matrix.md](compat-matrix.md).

## Layer 5: community hardware reports

`sol doctor` produces a redacted JSON report. Reports come in through the hardware-report
issue template and feed the matrix. The project's real test lab is its users. The job of
the tooling is to make reporting take two minutes.

## Planned

- **Record/replay devices**: capture real NVML telemetry and errors to fixtures, then replay
  them in CI, so a bug seen on a user's laptop becomes a permanent regression test.
- **Driver-update canary**: a self-hosted runner with real GPUs that runs layers 3 and 4 on
  every new driver release, and opens a PR that updates the matrix.
- **Stability testing as a feature**: an automated undervolt/offset search that runs a stress
  load and watches for errors, so stability isn't left to "run a game for an hour".
