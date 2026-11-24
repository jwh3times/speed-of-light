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

## Layer 2: closed-loop simulation (hermetic, every PR)

`crates/sol-service/tests/service.rs` runs the real `Service` against `SimulatedGpu`. That
covers the thermal model, firmware auto-fan, the power cap and fault injection (ignored writes,
permission errors, dead sensors, failing fan writes, lost device). Simulated time advances in
1 s steps, so a 10-minute load test runs in milliseconds. These tests assert outcomes ("the
GPU never reaches slowdown"), not implementation details.

To add a fault, add a flag to `sim::Faults`, implement it in `SimulatedGpu`, and write a
service test that asserts the safe outcome.

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
