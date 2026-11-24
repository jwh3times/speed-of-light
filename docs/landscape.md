# Landscape (as of November 2026)

| Tool | Vendors | Strengths | Weaknesses |
| --- | --- | --- | --- |
| MSI Afterburner + RTSS | All (NVIDIA, AMD, some Intel) | The default for 15 years; the RTSS overlay is tolerated by anti-cheats; V/F curve editor | One developer; years between stable releases (4.6.6 stable only in Oct 2025); funding disruptions; closed source |
| mVolt+ | NVIDIA only | Very active (10 releases in about 4 weeks of Oct–Nov 2026); aggressive power/voltage unlocks; laptop work | NVIDIA-only; the most valuable features fight NVIDIA's driver changes (power control broke on 615.56, voltage on 621.01+); relies on RTSS for the overlay |
| NVIDIA App | NVIDIA | First-party; auto-tuning | Limited manual control; no fan curves of comparable depth |
| AMD Adrenalin | AMD | First-party, full tuning | AMD-only; tied to the driver install |
| ASUS GPU Tweak III, others | All | V/F curve editors | Vendor-branded; mixed reputation |
| Fan Control (Rem0o) | Fans (all) | Excellent fan curves; moved from WinRing0 to PawnIO | Not a tuning tool |

## Positioning

Speed of Light doesn't try to out-unlock mVolt. Power and voltage unlocks are exactly what
NVIDIA patches, and they're what damages cards. The pitch is:

1. **The safe default.** Verified writes, a compatibility matrix, fail-to-cooling fans, and a
   crash guard.
2. **Vendor-neutral.** One tool, and one profile format, across NVIDIA, AMD and Intel.
3. **Open and testable.** A public test suite, a public compatibility matrix, and reproducible
   hardware reports.
4. **Undervolt-first.** Most users want a cooler, quieter card, not a benchmark record. A
   built-in stability search is the feature nobody else does well.

## Sources

- TechPowerUp, "MSI Pushes Afterburner 4.6.6 Stable Update After Years of Beta" (Oct 2025)
- Tom's Hardware, "Nvidia's latest driver update breaks mVolt overclocking functionality"
- github.com/b00nz/mVolt README, releases and issues (read 2026-11-24)
- Slashdot / The Verge, "Windows Defender now flags WinRing0 driver" (Mar 2025)
- FanControl.Releases discussion #3017 (PawnIO migration)
