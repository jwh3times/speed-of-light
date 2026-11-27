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
