# A service config that can't be trusted blocks every write except a return to stock

The installed service reads its service config, `%ProgramData%\SpeedOfLight\config.toml` (#17),
once at startup. `sol-service install --config` checks the file and copies it there, as ADR 0002
does for the startup profile. A missing file means the defaults. A file present but untrusted (a
non-administrator could change it), unreadable or invalid does not stop the service and does not
fall back to the defaults: the service starts, runs its startup takeover and keeps monitoring, but
skips the startup profile and refuses every write except a return to stock (`reset`, and fans
handed to firmware control). It names the reason in a notice on each device in `sol status`, in
the log, and in the error each refused write returns. The file holds the user's policy caps, so
the one outcome that must never happen is a write the caps would have refused.

## Considered options

- **Fall back to the defaults.** The defaults have no policy caps, so a broken file would quietly
  remove the limits the user set and allow the writes they meant to forbid.
- **Refuse to start.** The service's recovery settings restart it, so it would loop. Monitoring
  would stop, `sol status` could not explain why, and no client could reset a GPU that an earlier
  run left tuned.
- **Refuse every write, including a reset.** Stock is inside every cap, and firmware control is
  the cooling-safe state, so blocking them protects nothing. It would only leave settings an
  earlier run applied stuck in place until someone uninstalls.

## Consequences

- A hand edit that breaks the file, or an ACL change on it, turns tuning off until `install` is
  run again with a good file. `install` rejects a bad file before copying it, so this state comes
  only from a change made after install.
- `docs/safety.md` carries the invariant, with the test that guards it.
