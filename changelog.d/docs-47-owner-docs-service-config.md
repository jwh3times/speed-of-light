### Changed

- `SECURITY.md`'s threat model now covers the service config. The service reads only the
  administrator-owned copy, and a copy a standard user could change, or one that can't be read or
  parsed, blocks every write except a return to stock, so a tampered file can't lift the policy
  caps. The PRD marks TUN-4 (policy caps) done, and `ROADMAP.md` lists the service config under
  "Done (0.3.0)" (#47)
