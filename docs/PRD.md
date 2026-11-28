# Speed of Light: Product Requirements

| | |
| --- | --- |
| **Status** | Draft v1, 2026-11-24 |
| **Owner** | Project maintainer |
| **Related** | [ROADMAP.md](../ROADMAP.md), [architecture](architecture.md), [safety](safety.md), [testing](testing.md), [landscape](landscape.md) |

## 1. Summary

Speed of Light is an open-source (AGPL-3.0) GPU tuning, fan control and monitoring tool for
Windows. The goal is to become the tool people mean when they say "just install Afterburner".
It wins on **trust**, not on unlocks:

- every setting is verified after it's applied;
- support for each driver is proven by community hardware reports, not assumed;
- fan control fails towards cooling;
- one tool covers NVIDIA, AMD and Intel.

## 2. Problem

- **The default tool is fragile.** MSI Afterburner is effectively a one-developer project. Its
  stable releases come years apart (4.6.6 went stable in Oct 2025 after a long beta), and its
  funding has been disrupted. Millions of users depend on it with no succession plan.
- **The alternatives are narrow or brittle.** Vendor apps lock you to one brand. Newer
  community tools (e.g. mVolt+) are NVIDIA-only. They build their headline features on
  driver behaviour that NVIDIA changes without notice: power control broke on 615.56, and
  voltage changes are accepted but not enforced from 621.01.
- **Tools lie by omission.** Users see "Applied" when the driver silently ignored the
  setting, or `0 °C` for a sensor that doesn't exist. They can't tell a working tool from a
  broken one.
- **Low-level drivers are a liability.** Tools that depended on the WinRing0 kernel driver
  were flagged by Microsoft Defender in 2025, and anti-cheat systems block similar drivers.

## 3. Goals

1. **G1, Safe by default.** No write without range checks, a driver-compatibility decision
   and read-back verification. Fans never end up slower than the user's curve or the
   firmware would run them.
2. **G2, Honest.** Missing data is shown as missing. Accepted-but-ignored settings are shown as
   not enforced.
3. **G3, Vendor-neutral.** One profile format and one UI across NVIDIA, AMD and Intel.
4. **G4, Covers what people use Afterburner for.** Monitoring, power, clocks, fan curves,
   profiles, an in-game overlay and frame-time metrics.
5. **G5, Fast to support new drivers.** Days, not months, from a driver release to a verified
   compatibility entry.
6. **G6, Low overhead.** Imperceptible while gaming.

## 4. Non-goals

- VBIOS flashing, or "software BIOS overrides" that persist beyond the session.
- Power or voltage unlocks beyond what the vendor API exposes (shunt-mod emulation, OCP
  raises).
- Kernel drivers, for GPU control or anything else.
- CPU or RAM overclocking. Monitoring CPU/board sensors may come later, read-only.
- Linux and macOS for now. The core is portable, but the first-class target is Windows 10/11.
- Being the first tool to support extreme overclocking on a new card.

## 5. Users

| Persona | Wants | Key requirements |
| --- | --- | --- |
| **Quiet-PC gamer** (the majority) | Cooler, quieter card with near-stock performance; "set and forget" | Power limit, fan curves, startup profile, crash guard, undervolt assistant |
| **Enthusiast tuner** | Clock offsets, V/F curve, benchmarks, per-game profiles | Offsets, V/F editor (M3+), stability testing, per-app profiles |
| **Laptop user** | Stop thermal throttling, manage noise | Correct capability detection; no false promises where the OEM locks controls |
| **Streamer / benchmarker** | FPS, frame times, 1 %/0.1 % lows on screen and in logs | Overlay, PresentMon metrics, logging/export |
| **Integrator** | Stream Deck buttons, scripts, home-automation | Stable, documented IPC protocol |
| **Hardware reporter** | Help the project with their card | One-command read-only report; a safe self-test |

## 6. Product principles

1. **Verify, don't trust.** The driver accepting a value proves nothing. Read it back.
2. **Fail towards cooling.** When in doubt, the firmware owns the fans.
3. **Missing means `n/a`.** Never invent a number.
4. **Users are the test lab.** Make contributing a hardware report take two minutes.
5. **No ring 0.** Only vendor-supported user-mode APIs.
6. **Quiet when gaming.** The service is small; the GUI is optional and can be closed.

## 7. Requirements

Priority: **P0** must ship for 1.0, **P1** should, **P2** later. Status reflects the current
`main` branch.

### 7.1 Monitoring

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| MON-1 | Per-GPU temperature, core/memory clock, power draw and limit, utilization, VRAM, fan duty and RPM, P-state | P0 | Done (NVIDIA) |
| MON-2 | Performance limiters (power, thermal, idle, …) shown as reasons, not raw bitmasks | P0 | Done (NVIDIA) |
| MON-3 | Sensors the driver doesn't expose are reported as absent | P0 | Done |
| MON-4 | Hotspot and memory-junction temperature where a vendor API exposes them | P1 | Planned |
| MON-5 | Logging to CSV/JSON with a configurable interval | P1 | Planned |
| MON-6 | History graphs in the GUI | P0 | Planned (M2) |

### 7.2 Tuning

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| TUN-1 | Power limit as a percentage of stock or as watts, within driver range | P0 | Implemented; awaiting hardware verification |
| TUN-2 | Core and memory clock offsets within driver range | P0 | Implemented; awaiting hardware verification |
| TUN-3 | Reset to stock that always works, even when tuning is known broken | P0 | Done |
| TUN-4 | User policy caps (max power %, max offsets) inside driver range | P1 | Done (the service config's `max_power_percent`, `max_core_offset_mhz` and `max_memory_offset_mhz`; they bind the startup profile and every request) |
| TUN-5 | V/F curve editor (NVIDIA via NVAPI, AMD/Intel via their SDKs): opt-in, matrix-gated, verified | P1 | Planned (M3) |
| TUN-6 | Undervolt assistant: guided search with a stress load and error detection, producing a stability report | P1 | Planned (M3) |

### 7.3 Fans

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| FAN-1 | Firmware auto, fixed duty and temperature curves | P0 | Done |
| FAN-2 | Curve hysteresis, ramp-rate limits and critical-temperature override | P0 | Done |
| FAN-3 | Hand back to firmware on sensor loss, repeated write failure, service stop and service start | P0 | Done |
| FAN-4 | Fixed duty overridden before thermal slowdown | P0 | Done |
| FAN-5 | Duty below the driver minimum is raised, never lowered | P0 | Done |
| FAN-6 | Per-fan curves on multi-fan cards | P2 | Planned |
| FAN-7 | Curve sources other than GPU die temperature (hotspot, memory) | P2 | Planned |

### 7.4 Safety and verification

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| SAF-1 | Every write is range-checked, matrix-gated, then read back; outcomes are Applied / NotEnforced / Rejected / Blocked / Failed | P0 | Done |
| SAF-2 | Crash guard: a profile that didn't survive its stability window is not re-applied at boot | P0 | Done |
| SAF-3 | Warn when other tuning tools are running (and refuse self-test) | P0 | Done |
| SAF-4 | Run as a Windows service with automatic restart, so a killed process doesn't strand fans | P0 | Done |
| SAF-5 | Each safety invariant has a test that fails without it | P0 | Done; [safety.md](safety.md) is the index |

### 7.5 Compatibility matrix

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| CMP-1 | Matrix of (vendor, feature, driver range, architecture) → verified / broken, compiled in, with evidence required | P0 | Done |
| CMP-2 | Unverified combinations are opt-in; broken ones are blocked with the reason | P0 | Done |
| CMP-3 | `sol doctor` produces a redacted hardware report | P0 | Done |
| CMP-4 | `sol selftest --write` exercises and restores each control and emits matrix entries | P0 | Done |
| CMP-5 | Matrix updates ship without a full release (signed data update) | P1 | Planned |
| CMP-6 | A driver-update canary on real hardware opens matrix PRs automatically | P1 | Planned |

### 7.6 Profiles

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| PRO-1 | Human-editable TOML profiles, strictly parsed, schema-versioned | P0 | Done |
| PRO-2 | Device targeting by ID or name | P0 | Done |
| PRO-3 | Startup profile | P0 | Done (`--startup-profile` on `run` and `install`; the installed service applies an administrator-owned copy) |
| PRO-4 | Hotkeys and per-application profiles (switch on game launch) | P1 | Planned |
| PRO-5 | Import Afterburner profiles where they map cleanly | P2 | Planned |

### 7.7 Overlay and frame metrics

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| OVL-1 | FPS, frame time and 1 %/0.1 % lows via PresentMon (ETW, no injection) | P0 | Planned (M2) |
| OVL-2 | RTSS shared-memory integration for users who already run RTSS | P0 | Planned (M3) |
| OVL-3 | Native non-injecting overlay for borderless/windowed games | P1 | Planned (M3) |
| OVL-4 | Benchmark capture with summary export | P1 | Planned |

### 7.8 GUI

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| GUI-1 | Thin client over IPC that runs unelevated | P0 | Planned (M2) |
| GUI-2 | Dashboard, tuning panel, fan curve editor (drag points), profile manager | P0 | Planned (M2) |
| GUI-3 | Apply results shown per item, with NotEnforced and Blocked explained in plain language | P0 | Planned (M2) |
| GUI-4 | Tray icon; closes to tray; zero GPU usage when hidden | P0 | Planned (M2) |
| GUI-5 | Keyboard-accessible, screen-reader labels, high-contrast support | P1 | Planned |

### 7.9 Integration and distribution

| ID | Requirement | Pri | Status |
| --- | --- | --- | --- |
| INT-1 | Documented, versioned JSON-lines protocol over a named pipe | P0 | Done |
| INT-2 | Pipe restricted to SYSTEM, admins and interactive users; no remote clients | P0 | Done |
| INT-3 | Clients verify the pipe server's identity | P1 | Planned |
| DST-1 | Signed installer and binaries | P0 | Planned |
| DST-2 | winget package | P1 | Planned |
| DST-3 | No telemetry by default; any future opt-in telemetry is documented and redacted | P0 | Done (none exists) |

## 8. Non-functional requirements

These are **targets**, not yet measured. A benchmark harness to measure them is part of M1.

| Area | Target |
| --- | --- |
| Service CPU | < 0.5 % of one core at a 1 Hz tick with one GPU |
| Service memory | < 30 MB resident |
| IPC round trip | < 10 ms for `status` |
| Telemetry tick | Configurable 250 ms – 5 s; default 1 s |
| Start-up takeover | Service restart returns fans to firmware within 5 s of launch |
| Platforms | Windows 10 22H2+ and Windows 11, x86-64 (ARM64 later) |
| Security | No kernel components; elevated surface limited to the service; all IPC input validated |
| Privacy | Hardware reports hash device identifiers; nothing leaves the machine unless the user posts it |

## 9. Milestones

| Milestone | Scope | Exit criteria |
| --- | --- | --- |
| **M0 Foundation** (done) | Core, NVML backend, service, CLI, simulator, matrix, docs | Hermetic test suite green; telemetry verified on one real GPU |
| **M1 Verified NVIDIA** | Windows service install, config file, selftest on ≥ 4 NVIDIA architectures including one laptop, record/replay, perf harness | Writes verified on Turing, Ampere, Ada, Blackwell; zero unrestored-state reports |
| **M2 Public beta** | GUI, PresentMon metrics, logging, signed builds | 100 external hardware reports; GUI covers MON/TUN/FAN/PRO P0 |
| **M3 1.0: "replace Afterburner"** | Overlay (RTSS integration and native), undervolt assistant, per-app profiles, V/F curve (opt-in) | All P0 requirements done; recommended in at least one major tuning guide |
| **M4 Vendor-neutral** | AMD (ADLX) and Intel (IGCL) backends | AMD and Intel at parity with NVIDIA P0s; matrix entries for each |

## 10. Success metrics

- **Coverage:** distinct verified (architecture × driver × feature) entries in the matrix.
- **Driver latency:** median time from an NVIDIA/AMD/Intel driver release to a verified or
  broken matrix entry. Target under 72 hours by M3.
- **Safety:** reports of unrestored settings or fan-related overheating. Target zero.
  Every report becomes a regression test.
- **Contribution:** hardware reports per month; share of the matrix contributed externally.
- **Adoption:** downloads, winget installs, GitHub stars, and appearances in third-party
  tuning guides.

## 11. Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Vendors restrict APIs (NVIDIA already does) | Features break | Use documented APIs first; the matrix turns breakage into a clear message; reset always works |
| Anti-cheat blocks the overlay | Overlay unusable in popular games | PresentMon (no injection); RTSS integration; a non-injecting overlay |
| A user's card is damaged | Reputation, liability | Driver-range limits, policy caps, verification, fail-to-cooling fans, a clear disclaimer; no voltage unlocks |
| Bus factor (the very problem we criticize) | Project stalls | Open governance, AGPL, documented architecture, agent-readable docs, a test suite that lets new maintainers change code safely |
| AGPL deters vendor or commercial contributions | Slower AMD/Intel backends | Accept: the license keeps rebrands open. Revisit only with a concrete partner |
| Code-signing cost and SmartScreen reputation | Download friction | Community-funded certificate; winget; reproducible builds |
| Trademark confusion with Afterburner | Legal | Never use "Afterburner" in branding; state non-affiliation |

## 12. Open questions

1. GUI stack: Tauri vs WinUI 3. Decide with a two-week prototype of the fan-curve editor.
2. How far to go with NVAPI V/F curve and voltage given NVIDIA's enforcement changes.
3. Whether to add opt-in crash/compatibility telemetry, and what exactly it would collect.
4. Governance model once outside maintainers join.
