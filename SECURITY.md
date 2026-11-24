# Security

## Reporting

Please report vulnerabilities privately through GitHub Security Advisories on this repository,
not in public issues.

## Threat model

`sol-service` runs elevated and can change GPU power, clocks and fans. What protects it:

- **Pipe DACL.** `\\.\pipe\speed-of-light` grants full access to SYSTEM and Administrators,
  and read/write to Interactive Users (the person at the keyboard, including unelevated). It
  doesn't admit network logons or other service accounts, and remote clients are rejected.
  Any program the interactive user runs can therefore change GPU settings within the safety
  policy. That's the same trust level as Afterburner, made explicit here.
- **Anti-squatting.** The service creates the pipe with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so it
  refuses to start if another process already owns the name. That stops it from serving
  alongside an impostor.
- **Input limits.** Messages are capped at 1 MiB. Malformed JSON gets an error response.
  Profiles are strictly validated (unknown keys, ranges, schema version) before anything else
  happens. Every value is range-checked against the driver's reported limits.
- **No kernel driver.** Only vendor user-mode APIs (NVML). Nothing ring-0 ships with this
  project.
- **Hardware reports are redacted.** `sol doctor` replaces GPU UUIDs with a hash.

## Known gaps

- Clients don't yet verify that the process serving the pipe is the genuine, elevated service.
  A malicious unelevated process that starts *before* the service could impersonate it to
  clients. It still couldn't touch hardware, but it could lie about status. Tracked in
  ROADMAP.md.
- Release binaries aren't signed yet.
