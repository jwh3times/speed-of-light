# Speed of Light

[![CI](https://github.com/jwh3times/speed-of-light/actions/workflows/ci.yml/badge.svg)](https://github.com/jwh3times/speed-of-light/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

**Open, vendor-neutral GPU tuning, fan control and monitoring for Windows. It's built to be the
safe default that replaces MSI Afterburner.**

> **Status: pre-alpha.** NVIDIA monitoring works and is verified on real hardware. The service
> installs as a Windows service. Power limit,
> clock offset and fan control are implemented and tested against a simulated GPU, but they
> haven't been verified on any driver yet, so they stay blocked unless you opt in with
> `--allow-unverified`. There's no GUI or in-game overlay yet.
> See the [PRD](docs/PRD.md) and [ROADMAP](ROADMAP.md).

## Why

Afterburner is maintained by essentially one developer, and its stable releases arrive years
apart. The newer tools are single-vendor, and they break whenever a driver changes underneath
them. Speed of Light competes on **trust**:

| | |
| --- | --- |
| **Verified writes** | Every setting is read back after it's applied. If the driver accepts a value but ignores it, you see `NOT ENFORCED`, not "Applied". |
| **Proven driver support** | A [compatibility matrix](docs/compat-matrix.md) built from community hardware reports. Unverified driver/GPU combinations are opt-in. Known-broken ones are blocked with the reason. |
| **Fails toward cooling** | Fan curves hand control back to the GPU firmware if the sensor dies, if writes fail, or if the service stops. A fixed fan speed is overridden before thermal slowdown. [Every guarantee has a test.](docs/safety.md) |
| **Crash guard** | A profile that crashed your PC isn't re-applied at the next boot. |
| **Honest readings** | A sensor your card doesn't have shows `n/a`, never `0 °C`. |
| **No kernel driver** | Only vendor-supported user-mode APIs, so there's nothing for Defender or anti-cheat to flag. |
| **Vendor-neutral** | NVIDIA today. AMD (ADLX) and Intel (IGCL) plug into the same interface. |

## Features

| Area | Now | Planned |
| --- | --- | --- |
| Monitoring | Temps, clocks, power, utilization, VRAM, fans, throttle reasons (NVIDIA) | Hotspot/memory temps, logging, graphs |
| Tuning | Power limit, core/memory offsets (opt-in until verified), reset to stock | V/F curve editor, undervolt assistant |
| Fans | Auto, fixed, curves with hysteresis, ramp limits, critical override, watchdogs | Per-fan curves, other temperature sources |
| Profiles | Strict TOML, device targeting, startup profile | Hotkeys, per-game profiles |
| Overlay | n/a | PresentMon frame metrics, RTSS integration, native overlay |
| Interfaces | `sol` CLI, JSON-lines IPC | GUI (tray app) |
| Vendors | NVIDIA (NVML) | AMD (ADLX), Intel (IGCL) |

## Quick start

Needs Windows 10/11 (x64). NVIDIA monitoring uses the NVML library that ships with the driver,
so there's nothing else to install.

### Download a release

Every merge to `main` is a release. Each [GitHub Release](https://github.com/jwh3times/speed-of-light/releases)
has `speed-of-light-v<version>-windows-x64.zip`, built by CI from that release's tagged commit,
and a `SHA256SUMS.txt` beside it. The zip holds `sol.exe`, `sol-service.exe`,
`examples\profiles\`, `examples\config.toml`, this README, the license, `SECURITY.md` and the
release notes.

```powershell
# In the folder you downloaded both files to
(Get-FileHash .\speed-of-light-v<version>-windows-x64.zip -Algorithm SHA256).Hash.ToLower()   # compare with SHA256SUMS.txt
Expand-Archive .\speed-of-light-v<version>-windows-x64.zip -DestinationPath .
cd .\speed-of-light-v<version>-windows-x64
$env:Path = "$PWD;$env:Path"
```

### Or build from source

Needs [rustup](https://rustup.rs). `rust-toolchain.toml` pins Rust 1.88, which rustup installs on
the first build.

```powershell
git clone https://github.com/jwh3times/speed-of-light
cd speed-of-light
cargo build --release
$env:Path = "$PWD\target\release;$env:Path"
```

### Try it

```powershell
# Read-only, no service needed
sol probe                    # GPUs, capabilities, live readings, conflicting tools
sol doctor -o report.json    # redacted hardware report

# Try everything safely against a simulated GPU
sol-service run --simulate 1          # in one terminal
sol status                            # in another
sol apply --allow-unverified examples\profiles\quiet-curve.toml
sol watch
sol reset
```

To control a real GPU, install the service from an **elevated** terminal. Close Afterburner,
Fan Control and other tuning tools first: two tools driving the same fans fight each other.
`sol probe` lists any that are still running, and the service warns while one is.

```powershell
# Elevated. Installs (or upgrades) the Windows service and starts it.
sol-service install --config examples\config.toml --startup-profile examples\profiles\quiet-curve.toml
sol status                            # from any terminal

# Elevated. Stops it, returns every GPU to stock, and removes it.
sol-service uninstall                 # --purge also deletes the logs and crash guard
```

The service starts at boot and Windows restarts it within seconds if it dies. Each start hands
the fans back to firmware control before anything else. It runs from a copy in
`%ProgramFiles%\SpeedOfLight`. Its service config and startup profile are copies kept in
`%ProgramData%\SpeedOfLight` (with the logs), so to change either, edit your file and run
`install` again.

The service config (`examples\config.toml` shows every key) holds your policy caps (the highest
power limit and clock offsets the service will ever apply, whoever asks), the tick rate, the
crash guard's stability window, the log level and `allow_unverified`. Without `--config` the
service runs on the defaults, with no caps. If the installed copy can't be used (edited into
something invalid, or changeable by a standard user), the service still runs and monitors but
refuses every write except `sol reset` and `sol fan auto`, and `sol status` says why. Until your
GPU and driver are in the compatibility matrix, the startup profile's power, clock and fan
settings stay blocked unless the config sets `allow_unverified = true`, and the service log shows
the profile as not fully applied. To try the service without touching your GPU, install with
`--simulate 1`. For a console session instead, use `sol-service run` from an elevated terminal
while the service isn't running; it takes `--config` too, plus `--tick-ms`, `--stability-secs`,
`--log-level` and `--allow-unverified`, which win over the file.

### Profiles

```toml
schema = 1
name = "Quiet curve"

[power]
percent = 90                 # of the card's stock limit (or: watts = 300)

[clocks]
memory_offset_mhz = 500

[fan]
mode = "curve"               # "auto" | "fixed" | "curve"
points = [[40, 30], [55, 38], [65, 50], [75, 75], [83, 100]]
```

`sol profile check <file>` validates a profile without applying it. Unknown keys are errors,
so a typo can't silently do nothing.

### Scripts

Scripts should use `--json`, which every command takes: it carries temperatures, clocks and power
as numbers. The text output is UTF-8. A PowerShell console on its default code page decodes
captured text with that code page, so `88 °C` comes out as `88 ┬░C`. Set
`[Console]::OutputEncoding = [Text.Encoding]::UTF8` before capturing it.

## Help verify your GPU

Hardware reports are the most valuable contribution, and they don't need any code:

1. Close other tuning tools.
2. `sol doctor -o report.json` (read-only; GPU IDs are hashed).
3. Optional, from an elevated terminal: `sol selftest --write`. It nudges each control for a
   few seconds (power −10 %, core +15 MHz, memory +50 MHz, fans 60 %), verifies it, and
   restores the original.
4. [Open a hardware report](https://github.com/jwh3times/speed-of-light/issues/new?template=hardware-report.yml)
   with both outputs.

## How it's built

```
sol CLI / GUI / scripts ──JSON lines over a named pipe──► sol-service (elevated) ──► sol-core ──► vendor backends
```

| Crate | Role |
| --- | --- |
| [`sol-core`](crates/sol-core) | Domain model, safety gate (plan, compatibility check, write, verify), fan-curve engine, profiles, compatibility matrix, crash guard, GPU simulator. No hardware access. |
| [`sol-nvidia`](crates/sol-nvidia) | NVIDIA backend over NVML |
| [`sol-amd`](crates/sol-amd), [`sol-intel`](crates/sol-intel) | Detection only, for now |
| [`sol-ipc`](crates/sol-ipc) | Versioned protocol over a locked-down named pipe |
| [`sol-service`](crates/sol-service) | Owns the hardware: control loop, watchdogs, IPC server |
| [`sol-cli`](crates/sol-cli) | The `sol` command |

Documentation:

- [Product requirements](docs/PRD.md)
- [Architecture](docs/architecture.md)
- [Safety invariants](docs/safety.md)
- [Testing strategy](docs/testing.md)
- [Vendor APIs](docs/vendor-apis.md)
- [Landscape](docs/landscape.md)

## Development

```powershell
cargo test --workspace                                # hermetic: no GPU or admin needed
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test -p sol-nvidia -- --ignored --nocapture     # read-only live probe on an NVIDIA GPU
```

See [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md). Security issues: see
[SECURITY.md](SECURITY.md).

## License

[GNU Affero General Public License v3.0 only](LICENSE) (`AGPL-3.0-only`). You may use, study,
modify and share this software. If you distribute it, or make a modified version available to
users over a network, you must release your source under the same license.

## Disclaimer

Changing GPU power, clocks and fan behaviour can cause instability, data loss or hardware
damage. You use this software at your own risk. It comes with no warranty (see sections 15–16 of
the license). Speed of Light isn't affiliated with or endorsed by NVIDIA, AMD, Intel or MSI.
"MSI Afterburner" is a trademark of Micro-Star International.
