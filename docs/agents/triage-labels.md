# Triage labels

The labels and what they mean are in `.agents/skills/open-pr/settings/triage-labels.md`. A skill's
role name is the GitHub label to apply.

## How this repository uses them

`needs-info` covers any missing evidence, not only an unanswered question to a reporter: a
behaviour that can only be designed once someone has observed it on a real GPU and driver waits
with `needs-info` for that hardware report.

`ready-for-human` means an agent cannot complete the work: it needs credentials, real hardware
that must be written to (`sol selftest --write`, `sol apply`), or a decision only the maintainer can
make. It is not a judgement about difficulty.

## Extra labels

- `blocked` — the issue is waiting on another issue or an external event and cannot be progressed
  now, so nobody should pick it up. It sits beside the five rather than replacing one: the triage
  label still says what the issue will need once unblocked. Apply it with the board's `Gate` set to
  `Depends on another issue`, `External wait` or `Upstream`, and remove it in the same change that
  clears the blocker.
- `hardware-report` — a GPU and driver report filed from the issue template. It sits beside the
  five: a report is evidence for the compatibility matrix, and carries `needs-triage` until a
  maintainer has read it.
