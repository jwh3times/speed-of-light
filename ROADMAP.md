# Roadmap

## Done (0.2.0, pre-alpha)

- Runs as a real Windows service (`sol-service install` / `uninstall`): LocalSystem, starts at
  boot, restarted within seconds if it dies, and trusting only administrator-owned files
  ([ADR 0002](docs/adr/0002-service-trusts-only-admin-owned-state.md)). Waits for the GPU driver
  at boot, and reports tuning tools that start later (#16).

## Done (0.1.0, pre-alpha)

- Vendor-neutral core: units, capabilities, telemetry, profiles (TOML, strict).
- Safety gate: plan, then the compat matrix, then write, then read-back verification.
  `NotEnforced` detection.
- Fan-curve engine (hysteresis, ramp limits, critical override, sensor-loss fallback), with
  property tests.
- Simulated GPU with a thermal model and fault injection. Closed-loop service tests.
- NVIDIA NVML backend. Telemetry verified on RTX 3080 Ti / 621.01.
- Service with a control loop, fixed-fan watchdog, crash guard and startup fan takeover.
- Named-pipe IPC with a restrictive DACL and anti-squatting.
- `sol` CLI: probe, doctor (redacted report), selftest, status, watch, apply, reset, fan,
  guard, and profile check.
- Detection of conflicting tools (Afterburner, Fan Control, GPU Tweak, Precision X1, mVolt).

## Next: make NVIDIA writes trustworthy

- [ ] Run `sol selftest --write` on the maintainer's RTX 3080 Ti (with Afterburner and Fan
      Control closed). Commit the first `verified` write entries.
- [ ] Recruit testers for Turing, Ada, Blackwell and laptops. Hardware-report issue template.
- [ ] Record/replay device for turning user bug reports into CI regression tests.
- [x] Config file (`%ProgramData%\SpeedOfLight\config.toml`) for policy, startup profile and
      tick rate (#17, [ADR 0003](docs/adr/0003-a-bad-service-config-blocks-every-write.md)).
- [ ] Client-side verification of the pipe server's identity (`GetNamedPipeServerProcessId`
      plus a token/elevation check).
- [ ] Profile store and per-application profiles (switch on game launch).

## Then: the things people actually install Afterburner for

- [ ] GUI. Thin client over IPC; Tauri or WinUI 3 (decide with prototypes).
- [ ] Frame metrics through PresentMon (FPS, frame time, 1 %/0.1 % lows) with no injection.
- [ ] Overlay: RTSS shared-memory integration first, then a native non-injecting overlay for
      borderless games.
- [ ] Undervolt assistant: stress load, error detection, binary search on offsets, and a
      stability report.

## Later

- [ ] AMD backend (ADLX). Needs an RDNA contributor.
- [ ] Intel backend (IGCL). Needs an Arc contributor.
- [ ] NVIDIA V/F curve and voltage through NVAPI: opt-in, matrix-gated, read-back verified.
- [ ] Driver-update canary on a self-hosted runner with real GPUs.
- [ ] Signed releases (code-signing certificate), winget package, reproducible builds.

## Decisions needed

- ~~**License.**~~ Decided: AGPL-3.0-only (2026-11-24).
- **GUI stack.** Tauri (web UI, cross-platform-ish, larger runtime) vs. WinUI 3 (native, C#).
- **Name and branding check** before going public.
