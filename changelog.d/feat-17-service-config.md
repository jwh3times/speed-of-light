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
