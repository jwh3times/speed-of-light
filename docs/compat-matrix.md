# Compatibility matrix

`compat/matrix.toml` records which (vendor, feature, driver range, architecture)
combinations have been **verified** on real hardware, and which are **known broken**. It's
compiled into `sol-core`, so every build carries the matrix it was tested with.

## How a write is decided

For each action (`power_limit`, `clock_offset`, `fan_control`), `tuner::compat_gate` looks up
the device's vendor, driver version and architecture:

| Match | Result |
| --- | --- |
| Any `broken` entry | Blocked, always. The entry's note is shown. |
| A `verified` entry and no broken one | Allowed. |
| Nothing | Unverified: blocked unless `--allow-unverified` (or the service config's `allow_unverified = true`). |

Resetting to stock is never gated.

Driver versions compare numerically by component (`621.9` < `621.10`). A bounded entry never
matches a device whose driver version couldn't be read.

## Adding an entry

1. Run `sol doctor -o report.json` and `sol selftest --write` on the hardware. Use an elevated
   terminal for the selftest, with other tuning tools closed.
2. Open a hardware-report issue with both outputs.
3. Paste the entries `selftest` printed into `compat/matrix.toml`. Set `evidence` to the issue
   URL, and widen `min_driver` / `max_driver` only as far as you have evidence for.
4. `cargo test -p sol-core` checks the file parses and every entry has evidence.

A `broken` entry should say what happens in its `note` (e.g. "accepted but not enforced: reads
back +0 MHz").
