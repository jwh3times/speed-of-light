//! Tuning profiles: human-editable TOML, strictly parsed.
//!
//! ```toml
//! schema = 1
//! name = "Quiet 90%"
//!
//! [target]
//! name_contains = "3080"
//!
//! [power]
//! percent = 90          # of the card's default limit; or `watts = 300`
//!
//! [clocks]
//! core_offset_mhz = 0
//! memory_offset_mhz = 500
//!
//! [fan]
//! mode = "curve"        # "auto" | "fixed" | "curve"
//! points = [[40, 30], [60, 45], [75, 70], [85, 100]]
//! ```
//!
//! Unknown keys are an error. A typo like `core_ofset_mhz` must not silently do nothing.

use crate::device::{DeviceId, DeviceInfo, PowerLimitCaps};
use crate::fan::{CurveError, FanCurve, FanTuning};
use crate::units::{Milliwatts, Percent};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub target: DeviceMatch,
    #[serde(default)]
    pub power: Option<PowerSetting>,
    #[serde(default)]
    pub clocks: ClockSettings,
    #[serde(default)]
    pub fan: Option<FanSetting>,
}

/// Which devices a profile applies to. Empty matches every device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceMatch {
    pub id: Option<DeviceId>,
    pub name_contains: Option<String>,
}

impl DeviceMatch {
    pub fn matches(&self, info: &DeviceInfo) -> bool {
        if let Some(id) = &self.id
            && *id != info.id
        {
            return false;
        }
        if let Some(needle) = &self.name_contains
            && !info.name.to_lowercase().contains(&needle.to_lowercase())
        {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PowerSetting {
    /// Percentage of the card's default (stock) power limit.
    Percent(f32),
    Watts(f32),
}

impl PowerSetting {
    pub fn resolve(&self, caps: &PowerLimitCaps) -> Milliwatts {
        match *self {
            PowerSetting::Percent(p) => {
                Milliwatts(((caps.default.0 as f64) * (p as f64) / 100.0).round() as u32)
            }
            PowerSetting::Watts(w) => Milliwatts::from_watts(w),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockSettings {
    pub core_offset_mhz: Option<i32>,
    pub memory_offset_mhz: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum FanSetting {
    Auto,
    Fixed {
        duty: Percent,
    },
    Curve {
        /// `[temp_c, duty_percent]` pairs.
        points: Vec<(f32, f32)>,
        #[serde(default)]
        tuning: FanTuning,
    },
}

impl FanSetting {
    pub fn curve(&self) -> Option<Result<FanCurve, CurveError>> {
        match self {
            FanSetting::Curve { points, .. } => Some(FanCurve::from_pairs(points)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProfileError {
    #[error("profile is not valid TOML: {0}")]
    Parse(String),
    #[error("profile is invalid:\n  - {}", .0.join("\n  - "))]
    Invalid(Vec<String>),
}

impl Profile {
    pub fn from_toml(text: &str) -> Result<Profile, ProfileError> {
        let profile: Profile = toml::from_str(text).map_err(|e| ProfileError::Parse(e.to_string()))?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn to_toml(&self) -> Result<String, ProfileError> {
        toml::to_string_pretty(self).map_err(|e| ProfileError::Parse(e.to_string()))
    }

    /// Device-independent checks. Range checks against a real card happen in the tuner.
    pub fn validate(&self) -> Result<(), ProfileError> {
        let mut issues = Vec::new();
        if self.schema != SCHEMA_VERSION {
            issues.push(format!(
                "schema {} is not supported (this build reads schema {SCHEMA_VERSION})",
                self.schema
            ));
        }
        if self.name.trim().is_empty() {
            issues.push("name must not be empty".into());
        }
        match self.power {
            Some(PowerSetting::Percent(p)) if !(p.is_finite() && p > 0.0 && p <= 200.0) => {
                issues.push(format!("power.percent = {p} must be in (0, 200]"));
            }
            Some(PowerSetting::Watts(w)) if !(w.is_finite() && w > 0.0 && w <= 2000.0) => {
                issues.push(format!("power.watts = {w} must be in (0, 2000]"));
            }
            _ => {}
        }
        if let Some(Err(e)) = self.fan.as_ref().and_then(FanSetting::curve) {
            issues.push(format!("fan curve: {e}"));
        }
        if let Some(FanSetting::Curve { tuning, .. }) = &self.fan {
            if !(tuning.hysteresis.0.is_finite() && (0.0..=20.0).contains(&tuning.hysteresis.0)) {
                issues.push("fan.tuning.hysteresis must be in 0..=20 °C".into());
            }
            if !(tuning.ramp_down_per_sec.is_finite() && tuning.ramp_down_per_sec > 0.0) {
                issues.push("fan.tuning.ramp_down_per_sec must be > 0".into());
            }
            if let Some(up) = tuning.ramp_up_per_sec
                && !(up.is_finite() && up > 0.0)
            {
                issues.push("fan.tuning.ramp_up_per_sec must be > 0".into());
            }
            if !(tuning.critical_temp.0.is_finite() && (50.0..=110.0).contains(&tuning.critical_temp.0)) {
                issues.push("fan.tuning.critical_temp must be in 50..=110 °C".into());
            }
            if tuning.sensor_fail_ticks == 0 {
                issues.push("fan.tuning.sensor_fail_ticks must be at least 1".into());
            }
        }
        if issues.is_empty() { Ok(()) } else { Err(ProfileError::Invalid(issues)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Vendor;

    const FULL: &str = r#"
schema = 1
name = "Quiet 90%"

[target]
name_contains = "3080"

[power]
percent = 90

[clocks]
core_offset_mhz = 0
memory_offset_mhz = 500

[fan]
mode = "curve"
points = [[40, 30], [60, 45], [75, 70], [85, 100]]
"#;

    fn info(name: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId("nvidia:GPU-test".into()),
            vendor: Vendor::Nvidia,
            name: name.into(),
            architecture: None,
            driver_version: None,
            vbios_version: None,
            pci_bus_id: None,
        }
    }

    #[test]
    fn parses_full_profile() {
        let p = Profile::from_toml(FULL).unwrap();
        assert_eq!(p.power, Some(PowerSetting::Percent(90.0)));
        assert_eq!(p.clocks.memory_offset_mhz, Some(500));
        assert!(matches!(p.fan, Some(FanSetting::Curve { .. })));
        assert!(p.target.matches(&info("NVIDIA GeForce RTX 3080 Ti")));
        assert!(!p.target.matches(&info("NVIDIA GeForce RTX 4090")));
    }

    #[test]
    fn example_profiles_parse() {
        for (name, text) in [
            ("quiet-curve", include_str!("../../../examples/profiles/quiet-curve.toml")),
            ("memory-oc", include_str!("../../../examples/profiles/memory-oc.toml")),
        ] {
            if let Err(e) = Profile::from_toml(text) {
                panic!("examples/profiles/{name}.toml: {e}");
            }
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let p = Profile::from_toml(FULL).unwrap();
        let again = Profile::from_toml(&p.to_toml().unwrap()).unwrap();
        assert_eq!(p, again);
    }

    #[test]
    fn rejects_typos() {
        let text = FULL.replace("core_offset_mhz", "core_ofset_mhz");
        assert!(matches!(Profile::from_toml(&text), Err(ProfileError::Parse(_))));
    }

    #[test]
    fn rejects_unknown_schema_and_bad_curve() {
        let text = FULL.replace("schema = 1", "schema = 2").replace("[60, 45]", "[60, 20]");
        let Err(ProfileError::Invalid(issues)) = Profile::from_toml(&text) else {
            panic!("expected Invalid");
        };
        assert_eq!(issues.len(), 2, "{issues:?}");
    }

    #[test]
    fn minimal_profile_and_fixed_fan() {
        let p =
            Profile::from_toml("schema = 1\nname = \"fans\"\n[fan]\nmode = \"fixed\"\nduty = 55\n").unwrap();
        assert_eq!(p.fan, Some(FanSetting::Fixed { duty: Percent::new(55.0).unwrap() }));
        assert!(p.target.matches(&info("anything")));
        assert!(
            Profile::from_toml("schema = 1\nname = \"x\"\n[fan]\nmode = \"fixed\"\nduty = 101\n").is_err()
        );
    }

    #[test]
    fn power_resolves_against_default() {
        let caps = PowerLimitCaps {
            min: Milliwatts(100_000),
            max: Milliwatts(400_000),
            default: Milliwatts(350_000),
        };
        assert_eq!(PowerSetting::Percent(90.0).resolve(&caps), Milliwatts(315_000));
        assert_eq!(PowerSetting::Watts(300.5).resolve(&caps), Milliwatts(300_500));
    }
}
