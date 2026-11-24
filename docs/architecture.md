# Architecture

## Processes

```
  sol (CLI)  ─┐                         ┌─ sol-nvidia  ── nvml.dll
  GUI (later) ├─ JSON lines over ──► sol-service ── sol-core ─┼─ sol-amd     ── (ADLX, planned)
  scripts    ─┘  \\.\pipe\speed-of-light      (elevated)       └─ sol-intel   ── (IGCL, planned)
```

- **`sol-service`** is the only process that writes to GPUs. It runs elevated, because NVML
  writes need administrator rights. It owns the control loop (telemetry, fan curves,
  watchdogs, crash guard) and serves clients. Clients run unelevated.
- **Clients** (`sol`, a future GUI, Stream Deck plugins, scripts) talk to the service over a
  named pipe. They never touch hardware, except `sol probe` / `doctor` (read-only) and
  `selftest` (evidence gathering).
- Closing the GUI doesn't stop fan curves, and the GUI uses no resources while you play.

## Data flow for an apply

1. Client sends `ApplyProfile { profile, allow_unverified }`.
2. `tuner::plan` resolves the profile against the device's `Capabilities` and the user's
   `SafetyPolicy`. Out-of-range items are `Rejected`. It's pure, so nothing is written.
3. `Service::apply` arms the crash guard if power or clocks are involved.
4. `tuner::execute` checks each action against the compat matrix (`Blocked` if it fails),
   writes it, and reads it back (`Applied`, `NotEnforced`, or `Failed`).
5. A fan curve, if present and allowed, becomes a `FanController` run by `Service::tick`.
6. The client gets one `ApplyReport` per device, with a line per item.

## Why Rust

- The service runs elevated and parses input from other processes. Memory safety matters there.
- Backend FFI is small and isolated (`sol-nvidia` uses `nvml-wrapper`, Win32 calls live in
  `sol-ipc::pipe` and `sol-service::conflicts`).
- `Service` has no I/O of its own, so the whole control loop runs deterministically in tests
  against `SimulatedGpu`, with simulated time.

## Why JSON lines over a named pipe

Language-neutral and debuggable. The GUI can be written in anything (Tauri, WinUI, Avalonia)
without linking Rust code, and third-party integrations get a stable, documented protocol.
`PROTOCOL_VERSION` plus a `Hello` handshake guards compatibility.

## State

`%ProgramData%\SpeedOfLight\` holds `crash-guard.json`. Profiles are user files; the service
reads a startup profile path given on the command line. Config files and a profile store come
with the GUI milestone.
