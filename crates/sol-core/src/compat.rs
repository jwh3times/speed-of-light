//! The driver compatibility matrix: which (vendor, feature, driver, architecture) combinations
//! have been verified on real hardware, and which are known to be broken.
//!
//! The matrix ships in `compat/matrix.toml` and is compiled in. Each entry needs evidence (a
//! hardware report or `sol selftest` output). Writes on a combination with no entry are
//! *unverified*, and the tuner blocks them unless the user opts in. Known-broken entries are
//! always blocked. This turns "NVIDIA changed something in a driver" from silent breakage into
//! a clear message.

use crate::device::Vendor;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

const BUILTIN: &str = include_str!("../../../compat/matrix.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Telemetry,
    PowerLimit,
    ClockOffset,
    FanControl,
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Feature::Telemetry => "telemetry",
            Feature::PowerLimit => "power_limit",
            Feature::ClockOffset => "clock_offset",
            Feature::FanControl => "fan_control",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    Verified,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub vendor: Vendor,
    pub feature: Feature,
    pub status: EntryStatus,
    /// Inclusive lower bound, e.g. `"621.01"`. Absent means no lower bound.
    pub min_driver: Option<String>,
    /// Inclusive upper bound. Absent means no upper bound.
    pub max_driver: Option<String>,
    /// Architecture name as the backend reports it (e.g. `"Ampere"`). Absent matches all.
    pub architecture: Option<String>,
    pub note: String,
    /// Link or reference to the hardware report that backs this entry.
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatrixFile {
    #[serde(default, rename = "entry")]
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verdict {
    Verified { note: String },
    Broken { note: String },
    Unverified,
}

#[derive(Debug, Clone, Default)]
pub struct CompatMatrix {
    entries: Vec<Entry>,
}

impl CompatMatrix {
    pub fn builtin() -> Self {
        // The file is compiled in and covered by a test, so a parse failure is a build defect.
        #[allow(clippy::expect_used)]
        CompatMatrix::from_toml(BUILTIN).expect("compat/matrix.toml must parse")
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let file: MatrixFile = toml::from_str(text).map_err(|e| e.to_string())?;
        for e in &file.entries {
            for v in [&e.min_driver, &e.max_driver].into_iter().flatten() {
                if parse_version(v).is_none() {
                    return Err(format!("entry for {} {}: bad driver version {v:?}", e.vendor, e.feature));
                }
            }
            if e.evidence.trim().is_empty() {
                return Err(format!("entry for {} {} has no evidence", e.vendor, e.feature));
            }
        }
        Ok(CompatMatrix { entries: file.entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// A broken entry beats a verified one, so a regression report always wins.
    pub fn lookup(
        &self,
        vendor: Vendor,
        feature: Feature,
        driver: Option<&str>,
        architecture: Option<&str>,
    ) -> Verdict {
        let mut verified = None;
        for e in &self.entries {
            if e.vendor != vendor || e.feature != feature {
                continue;
            }
            if let Some(arch) = &e.architecture
                && architecture.is_none_or(|a| !a.eq_ignore_ascii_case(arch))
            {
                continue;
            }
            if !driver_in_range(driver, e.min_driver.as_deref(), e.max_driver.as_deref()) {
                continue;
            }
            match e.status {
                EntryStatus::Broken => return Verdict::Broken { note: e.note.clone() },
                EntryStatus::Verified => verified = Some(e.note.clone()),
            }
        }
        match verified {
            Some(note) => Verdict::Verified { note },
            None => Verdict::Unverified,
        }
    }
}

fn driver_in_range(driver: Option<&str>, min: Option<&str>, max: Option<&str>) -> bool {
    if min.is_none() && max.is_none() {
        return true;
    }
    let Some(d) = driver.and_then(parse_version) else {
        // A bounded entry can't vouch for a driver whose version we couldn't read.
        return false;
    };
    let ge_min = min.and_then(parse_version).is_none_or(|m| cmp_versions(&d, &m) != Ordering::Less);
    let le_max = max.and_then(parse_version).is_none_or(|m| cmp_versions(&d, &m) != Ordering::Greater);
    ge_min && le_max
}

/// `"621.01"` -> `[621, 1]`. Components compare numerically; missing ones count as 0.
pub fn parse_version(v: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = v.trim().split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

fn cmp_versions(a: &[u32], b: &[u32]) -> Ordering {
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        match x.cmp(&y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[entry]]
vendor = "nvidia"
feature = "power_limit"
status = "verified"
min_driver = "580.00"
architecture = "Ampere"
note = "ok"
evidence = "test"

[[entry]]
vendor = "nvidia"
feature = "power_limit"
status = "broken"
min_driver = "621.01"
max_driver = "621.10"
note = "regressed"
evidence = "test"
"#;

    #[test]
    fn builtin_matrix_parses() {
        let _ = CompatMatrix::builtin();
    }

    #[test]
    fn broken_beats_verified_and_ranges_are_inclusive() {
        let m = CompatMatrix::from_toml(SAMPLE).unwrap();
        let look = |d| m.lookup(Vendor::Nvidia, Feature::PowerLimit, Some(d), Some("Ampere"));
        assert!(matches!(look("600.5"), Verdict::Verified { .. }));
        assert!(matches!(look("621.01"), Verdict::Broken { .. }));
        assert!(matches!(look("621.10"), Verdict::Broken { .. }));
        assert!(matches!(look("621.11"), Verdict::Verified { .. }));
        assert_eq!(look("579.99"), Verdict::Unverified);
    }

    #[test]
    fn architecture_and_feature_must_match() {
        let m = CompatMatrix::from_toml(SAMPLE).unwrap();
        assert_eq!(
            m.lookup(Vendor::Nvidia, Feature::PowerLimit, Some("600"), Some("Ada")),
            Verdict::Unverified
        );
        assert_eq!(
            m.lookup(Vendor::Nvidia, Feature::FanControl, Some("600"), Some("Ampere")),
            Verdict::Unverified
        );
        assert_eq!(
            m.lookup(Vendor::Amd, Feature::PowerLimit, Some("600"), Some("Ampere")),
            Verdict::Unverified
        );
    }

    #[test]
    fn unknown_driver_version_never_matches_a_bounded_entry() {
        let m = CompatMatrix::from_toml(SAMPLE).unwrap();
        assert_eq!(m.lookup(Vendor::Nvidia, Feature::PowerLimit, None, Some("Ampere")), Verdict::Unverified);
        assert_eq!(
            m.lookup(Vendor::Nvidia, Feature::PowerLimit, Some("garbage"), Some("Ampere")),
            Verdict::Unverified
        );
    }

    #[test]
    fn rejects_entries_without_evidence_or_with_bad_versions() {
        assert!(CompatMatrix::from_toml(&SAMPLE.replace("evidence = \"test\"", "evidence = \"\"")).is_err());
        assert!(CompatMatrix::from_toml(&SAMPLE.replace("580.00", "580.x")).is_err());
    }

    #[test]
    fn version_compare_is_numeric() {
        assert_eq!(cmp_versions(&[621, 1], &[621, 10]), Ordering::Less);
        assert_eq!(cmp_versions(&[621], &[621, 0]), Ordering::Equal);
        assert_eq!(cmp_versions(&[1000, 0], &[999, 99]), Ordering::Greater);
    }
}
