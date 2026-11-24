# Guard

The commands and paths `scripts/guard.mjs` stops here, on top of the rules it applies everywhere.
Each `##` heading is one setting, defined in `.agents/skills/open-pr/settings/guard.md`. The ask
rules enforce `AGENTS.md` rule 5: nothing writes to the user's GPU without their word.

## Deny commands

- `^npx (-y |--yes )?skills update\b` — it leaves `.claude/skills/` linked to its source instead of
  copied. Re-run the install commands in the `shared-skills` block of `AGENTS.md` instead.

## Ask commands

- `^(sol|cargo run\b.*\s--)( --?[\w-]+( [^-\s]\S*)?)* selftest\b.*\s--write\b` — it writes every
  control on a real GPU to produce matrix evidence. Ask first, and check no other tuning tool is
  running (`sol probe` lists them).
- `^(sol|cargo run\b.*\s--)( --?[\w-]+( [^-\s]\S*)?)* (apply|reset|fan)\b` — it changes clocks,
  power or fans on whichever GPUs the service manages, which are real ones unless the service was
  started with `--simulate`.
- `^(sol-service|cargo run\b.*\s--) run\b(?!.*\s--simulate\b)` — it starts the service against real
  GPUs, with the fan loop writing duties. Use `--simulate <N>` unless the user asked for real
  hardware.

## Protected paths

- `^report\.json$` — a hardware report captured with `sol doctor -o`, not regenerable without the
  machine it came from.
