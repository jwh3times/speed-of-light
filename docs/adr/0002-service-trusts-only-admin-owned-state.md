# The service runs as LocalSystem from an admin-only copy and trusts only admin-owned state

`sol-service install` (#16) registers the service as LocalSystem, because NVML's set calls need
administrator rights and no unprivileged service account has them. A LocalSystem service runs
whatever executable its registration names and reads whatever files it is pointed at, so it
trusts nothing a standard user can write. `install` copies `sol-service.exe` into
`%ProgramFiles%\SpeedOfLight\` and registers that copy. It copies the startup profile into
`%ProgramData%\SpeedOfLight\` and sets that folder's owner to Administrators, with a protected ACL
(SYSTEM and Administrators full control, Users read, nothing inherited). At start, the service
skips the startup profile if a non-admin can write to it. To change the profile, you run
`install` again from an elevated terminal.

## Considered options

- **Register the executable where it was run from** (a release folder in `Downloads\`,
  `target\release\`). Any process running as that user could replace the file and run as SYSTEM
  at the next start.
- **Point the service at the user's own profile file.** Anyone who can edit that file would
  choose what the elevated service applies at every boot.
- **Rely on `%ProgramData%`'s inherited ACL.** It grants `BUILTIN\Users` the right to create
  folders, so a standard user could pre-create `SpeedOfLight\` before install and own the
  startup profile and the crash guard.

## Consequences

- `install` copies files, so it is also the upgrade path. A rebuilt binary or an edited profile
  takes effect only when `install` is run again.
- This protects what the service does at start, not who may tune at runtime. The pipe still
  accepts interactive non-admin users (INT-2), so they can apply profiles while the service runs.
  That is a separate decision.
