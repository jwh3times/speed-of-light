# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) via the
`v<major>.<minor>.<build>` tags minted by `.github/workflows/release.yml`. Branches never edit this
file: each writes its entries to `changelog.d/`, and the workflow stamps them into the section for
the version its merge mints ([ADR 0001](docs/adr/0001-stamp-changelog-on-tag.md)).

_`v0.0.0` marks the initial commit, which predates this scheme and has no section; its code is
released in 0.1.0._

## [Unreleased]

No unreleased changes.

## [0.3.1] - 2026-11-27

### Changed

- `SECURITY.md`'s threat model now covers the service config. The service reads only the
  administrator-owned copy, and a copy a standard user could change, or one that can't be read or
  parsed, blocks every write except a return to stock, so a tampered file can't lift the policy
  caps. The PRD marks TUN-4 (policy caps) done, and `ROADMAP.md` lists the service config under
  "Done (0.3.0)" (#47)

## [0.3.0] - 2026-11-27

### Added

- The installed service takes its settings from a service config,
  `%ProgramData%\SpeedOfLight\config.toml`. `sol-service install --config <file>` checks the file and copies it in, and a file that isn't
  valid leaves the install as it was. The config holds the policy caps (`max_power_percent`,
  `max_core_offset_mhz`, `max_memory_offset_mhz`), `allow_unverified`, the tick rate, the crash
  guard's stability window and the log level. It is parsed strictly: a schema version, no unknown
  keys and checked ranges. Run `install` again to change it; without `--config`, the service runs
  on the defaults, with no caps (#17)
- Policy caps now work. They bind the startup profile and every client's request, and a setting
  over a cap is refused, not lowered to it. A client's `--allow-unverified` can't loosen a cap (#17)
- If the installed service config can't be used (edited into something invalid, unreadable, or
  changeable by a standard user), the service still starts and monitors. It skips the startup
  profile and refuses every write except `sol reset` and `sol fan auto`, and `sol status` shows
  why at the top of each GPU's notices (#17)
- `sol-service run --config <file>` reads a service config in place. Its `--tick-ms`,
  `--stability-secs`, `--log-level` and `--allow-unverified` flags win over the file, key by
  key (#17)
- The release zip includes `examples\config.toml`, with every setting and its default (#17)

### Changed

- `sol-service install` no longer takes `--tick-ms`, `--stability-secs`, `--log-level` or
  `--allow-unverified`: put them in a service config and pass `--config`. The registered service
  command line is now just `service`, plus the `--simulate` flags for a simulated install.
  Uninstall still recognises a simulated install registered by an earlier version (#17)
- `--stability-secs` now takes 30 to 3600 seconds (#17)
- Uninstall also removes the installed service config (#17)

## [0.2.6] - 2026-11-27

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#42)

## [0.2.5] - 2026-11-27

### Fixed

- The README's new Scripts section explains why `°C` shows as `┬░C` when PowerShell captures
  `sol`'s text output on a console's default code page. `sol` writes UTF-8, and PowerShell decodes
  it with the console's code page. Scripts should use `--json`, which carries temperatures as
  numbers, or set `[Console]::OutputEncoding = [Text.Encoding]::UTF8` before capturing text (#38)

## [0.2.4] - 2026-11-27

### Fixed

- `sol status` and `sol watch` no longer end "can't reach the service" with
  `Caused by: The system cannot find the file specified. (os error 2)`, which read like a missing
  file. That error only means nothing is serving the pipe, which the message already explains. Any
  other error, such as access denied or the pipe being busy, is still shown as the cause (#39)

## [0.2.3] - 2026-11-27

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#13)

## [0.2.2] - 2026-11-27

### Changed

- `ROADMAP.md` lists the Windows service under "Done (0.2.0)", and `docs/testing.md` links the
  acceptance run's own wiki page.

## [0.2.1] - 2026-11-27

### Changed

- `SECURITY.md`'s threat model now covers the installed service: it runs only from an
  administrator-only copy, trusts only its administrator-owned state folder, and accepts pipe
  requests from boot for whoever is signed in. The PRD marks SAF-4 (run as a Windows service) done
  (#36)

## [0.2.0] - 2026-11-26

### Added

- `sol-service install` installs Speed of Light as the Windows service `SpeedOfLight` and starts
  it, from an elevated terminal. It starts at boot, and Windows restarts it 2 s, 2 s, then 10 s
  after it dies, so a killed service no longer leaves the fans at their last duty: each start
  hands them back to firmware control before anything else (#16)
- `install --startup-profile <file>` checks the profile, then applies an administrator-owned copy
  at every start. Run `install` again to change it, or to upgrade in place. It also takes
  `--allow-unverified`, `--tick-ms`, `--stability-secs` and `--log-level` (#16)
- `sol-service uninstall` stops the service, returns every GPU to stock, and removes it, exiting
  with an error when a GPU isn't verifiably back at stock. `--purge` also deletes the logs and the
  crash guard (#16)
- The service writes daily log files to `%ProgramData%\SpeedOfLight\logs`, keeping a week (#16)
- At boot the service waits for the GPU driver instead of giving up, and `sol status` says what it
  is waiting for (#16)
- A tuning or fan tool that starts after the service, such as Afterburner or Fan Control at login,
  is reported on every GPU (#16)

### Changed

- The IPC protocol is now version 2: `status` carries each backend's state. Clients that speak
  version 1 are refused at the handshake (#16)
- `sol probe` and `sol doctor` report a driver that isn't ready yet as `not_ready`, distinct from
  no driver at all (#16)
- When `sol` can't reach the service, it says whether the service isn't installed or is stopped,
  and how to fix it (#16)
- `sol-service run` refuses to open real GPUs while the installed service is running (#16)

### Security

- The service runs only from an administrator-only copy under `%ProgramFiles%\SpeedOfLight`.
  `install` takes `%ProgramData%\SpeedOfLight` back from whoever created it, since any user may
  create folders in `%ProgramData%`, and the service won't apply a startup profile a
  non-administrator could change (`docs/adr/0002-service-trusts-only-admin-owned-state.md`) (#16)

## [0.1.7] - 2026-11-26

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#12)

## [0.1.6] - 2026-11-26

### Changed

- The release settings ignore the `v0.0.0` tag, which marks the initial commit and has no changelog
  section, so the records audit no longer reports it as released without one (#9)

## [0.1.5] - 2026-11-25

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#10)

## [0.1.4] - 2026-11-25

### Changed

- The agent docs record that merges never edit `VERSION`, the `main` and `Version tags` rulesets in
  full, and that a pull request with no CodeQL analysis on its head cannot merge until a new commit
  starts one (#5)

## [0.1.3] - 2026-11-25

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#7)

## [0.1.2] - 2026-11-24

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#6)

## [0.1.1] - 2026-11-24

### Changed

- build(deps): Bump `toml` from `0.9.12+spec-1.1.0` to `1.2.2+spec-1.1.0` (#4)

## [0.1.0] - 2026-11-24

### Added

- First code, licensed AGPL-3.0-only. See `ROADMAP.md` under "Done" for the feature list. Writes on
  real hardware are implemented but not yet verified on any driver, so they're blocked unless
  `--allow-unverified` is passed.
- Each release has a Windows x64 zip on its GitHub Release, holding `sol.exe`, `sol-service.exe`,
  the example profiles, the README, the license and the release notes, with a `SHA256SUMS.txt`
  beside it.
- The shared agent skills from `jwh3times/skills` and their settings under `docs/agents/`.

### Changed

- Every merge to `main` is a release: `.github/workflows/release.yml` stamps this changelog from
  `changelog.d/` fragments and tags the merge `v<major>.<minor>.<build>`. `VERSION` holds the
  release line, and `scripts/check-cargo-version.mjs` keeps Cargo's workspace version in step.
- The Rust toolchain is pinned to 1.88, the minimum supported version, in `rust-toolchain.toml`.
- CI runs `rust` (format, lint, test on Windows), `agents` (the repository scripts) and
  `changelog`, with every action pinned by commit.
