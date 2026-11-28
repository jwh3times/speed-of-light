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
  - Installed (`sol-service install`), it is the Windows service `SpeedOfLight`. It runs as
    LocalSystem from an administrator-only copy under `%ProgramFiles%\SpeedOfLight`, starts at
    boot, and is restarted by Windows within seconds if it dies (ADR 0002).
  - In a console (`sol-service run`), it serves until Ctrl+C. Both run `host::run`; they
    differ only in what asks them to stop.
  - It serves the pipe from the moment it starts. Until a GPU is ready (the driver can still
    be loading at boot), it re-opens the backends with backoff, and `status` says what it is
    waiting for.
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
  `sol-ipc::pipe`, `sol-service::conflicts`, `sol-service::trust` and `sol-service::winsvc`).
- `Service` has no I/O of its own, so the whole control loop runs deterministically in tests
  against `SimulatedGpu`, with simulated time.

## Why JSON lines over a named pipe

Language-neutral and debuggable. The GUI can be written in anything (Tauri, WinUI, Avalonia)
without linking Rust code, and third-party integrations get a stable, documented protocol.
`PROTOCOL_VERSION` plus a `Hello` handshake guards compatibility.

## State

`%ProgramData%\SpeedOfLight\` holds `crash-guard.json`, the logs (`logs\`, daily files, seven
kept) and, when they are installed, `config.toml` (the service config) and
`startup-profile.toml`.

- `install` sets the folder's owner to Administrators and gives it a protected ACL:
  SYSTEM and Administrators full control, Users read. `C:\ProgramData` lets any user create
  folders, so this is done even when the folder already exists.
- The service config and the startup profile are copies `install` takes (`--config`,
  `--startup-profile`), checked first so a bad file changes nothing. Editing an original changes
  nothing until `install` runs again. The service won't apply a startup profile a
  non-administrator could change.
- The service reads its config once, at start. A missing one means the defaults, with no policy
  caps. One that is untrusted, unreadable or invalid doesn't stop the service: it runs and
  monitors, but refuses every write except a return to stock, and each GPU's notices say why
  (ADR 0003).
- The installed service's command line holds only `service` and, for a test install, the
  simulate flags.
- In a console, `run --config <file>` and `run --startup-profile <file>` read the named files
  directly, and `run`'s setting flags win over the config.
