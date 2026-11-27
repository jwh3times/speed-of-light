### Changed

- `SECURITY.md`'s threat model now covers the installed service: it runs only from an
  administrator-only copy, trusts only its administrator-owned state folder, and accepts pipe
  requests from boot for whoever is signed in. The PRD marks SAF-4 (run as a Windows service) done
  (#36)
