# Vendor APIs

What each vendor exposes, what this project uses, and what's missing. Items marked
*(to verify)* come from SDK documentation and haven't been exercised in this codebase yet.

## NVIDIA

### NVML (used)

`nvml.dll` ships with every driver. It's documented and supported, and accessed through the
`nvml-wrapper` crate.

| Capability | NVML call | Status here |
| --- | --- | --- |
| Temps, clocks, power, utilization, VRAM, P-state | `nvmlDeviceGet*` | Read, verified on RTX 3080 Ti / 621.01 |
| Throttle reasons | `nvmlDeviceGetCurrentClocksThrottleReasons` | Read, mapped to `PerfLimiter` |
| Fan duty / RPM / min-max | `nvmlDeviceGetFanSpeed_v2`, `...FanSpeedRPM`, `...MinMaxFanSpeed` | Read, verified |
| Power limit | `nvmlDeviceSetPowerManagementLimit` (admin) | Write implemented, unverified |
| Core / memory offset | `nvmlDeviceSetClockOffsets` (R555+, admin) | Write implemented, unverified |
| Fan duty | `nvmlDeviceSetFanSpeed_v2`, `...SetDefaultFanSpeed_v2` (admin) | Write implemented, unverified |
| Slowdown / shutdown temperature | `nvmlDeviceGetTemperatureThreshold` | Read; caps fan-curve critical temp |

Observed on RTX 3080 Ti, driver 621.01: core offset range −1000..=+1000 MHz, memory
−2000..=+6000 MHz, power 100–450 W (default 400 W), 2 fans with a 30 % minimum.

### Not in NVML

- **Per-point V/F curve editing** (the Afterburner Ctrl+F curve) and **voltage control**. Both
  need undocumented NVAPI entry points, reached via `nvapi_QueryInterface` with function IDs
  that the community has reverse-engineered. NVIDIA changes them without notice. mVolt's notes
  show voltage changes being accepted but not enforced from driver 621.01. A future
  `sol-nvidia-nvapi` feature should:
  - be opt-in, and off by default;
  - be gated by matrix entries per driver;
  - always be verified with read-back.
- **Hotspot and memory-junction temperature** on GeForce. NVML only reports the die
  temperature. Other tools read these through undocumented NVAPI thermal calls.

## AMD: ADLX (planned)

ADLX is AMD's public SDK, `amdadlx64.dll`, shipped with Adrenalin. It's C++ interfaces with a C
binding. Manual tuning lives under `IADLXGPUTuningServices`: manual graphics tuning
(frequency/voltage), VRAM tuning, fan tuning (curves, zero-RPM) and power tuning (power limit,
TDC). Feature availability varies by RDNA generation, so query `IsSupported*` per GPU
*(to verify)*. A backend needs RDNA hardware to develop against, so it's a good first
contribution for someone who has a card.

## Intel: IGCL (planned)

The Intel Graphics Control Library is `ControlLib.dll`, shipped with Arc drivers. It's a C API.
Overclocking is `ctlOverclock*` (frequency offset, voltage offset, power and temperature
limits). Fans are `ctlFan*` (fixed speed and speed tables) *(to verify)*.

## Frame metrics: PresentMon (planned)

For FPS, frame times and 1 %/0.1 % lows, the plan is Intel's open-source PresentMon. It reads
present events through ETW, so it measures without injecting into games. Injecting overlays get
flagged by anti-cheat, and RTSS is only tolerated because it has been around for over a decade.
An in-game overlay comes later. The options are:

- **RTSS shared memory**: optional integration, for users who already run RTSS.
- **A non-injecting topmost window**: for borderless and windowed games.
- **An injecting overlay**: only if the anti-cheat situation allows it.

## What we avoid

- **Kernel drivers** (WinRing0 and similar). Defender flags WinRing0 as a vulnerable driver.
  Nothing this tool needs for GPUs requires ring 0. If CPU/board sensors are ever added, use
  PawnIO, and note that some anti-cheats (FACEIT) block it.
- **VBIOS modification**, and "software BIOS overrides" that persist past the session. These
  are out of scope.
