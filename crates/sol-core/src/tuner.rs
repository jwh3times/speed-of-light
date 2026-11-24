//! The safety gate. Every write to a GPU goes through here.
//!
//! Applying a profile is two steps:
//!
//! 1. [`plan`] is pure: it resolves a profile against the device's capabilities and rejects
//!    anything out of the driver's range or the user's policy. Nothing is written.
//! 2. [`execute`] checks each action against the compatibility matrix, writes it, then reads
//!    it back. A write the driver accepted but didn't enforce is reported as
//!    [`Outcome::NotEnforced`]. Some drivers do this (see NVIDIA 621.xx voltage changes), and
//!    showing the user "applied" when nothing changed is the worst kind of failure.
//!
//! Fan *curves* aren't applied here. They need a control loop, so the plan hands the curve to
//! the service. Fixed duty and firmware-auto are applied here.

use crate::backend::{DeviceError, GpuDevice};
use crate::compat::{CompatMatrix, Feature, Verdict};
use crate::device::{Capabilities, ClockDomain, DeviceInfo};
use crate::fan::{FanCurve, FanTuning};
use crate::profile::{FanSetting, Profile};
use crate::units::{MhzOffset, Milliwatts, Percent};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Fan readback tolerance, in duty percentage points.
pub const FAN_TOLERANCE: f32 = 3.0;

/// User-level limits layered on top of whatever the driver allows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SafetyPolicy {
    /// Allow writes on driver/architecture combinations nobody has verified yet.
    pub allow_unverified: bool,
    /// Cap power limit at this percentage of the card's default, even if the driver allows
    /// more. `None` = driver maximum.
    pub max_power_percent: Option<f32>,
    /// Cap the absolute value of clock offsets. `None` = driver range.
    pub max_core_offset_mhz: Option<i32>,
    pub max_memory_offset_mhz: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    SetPowerLimit { limit: Milliwatts },
    SetClockOffset { domain: ClockDomain, offset: MhzOffset },
    SetFanDuty { duty: Percent },
    FansAuto,
}

impl Action {
    pub fn feature(&self) -> Feature {
        match self {
            Action::SetPowerLimit { .. } => Feature::PowerLimit,
            Action::SetClockOffset { .. } => Feature::ClockOffset,
            Action::SetFanDuty { .. } | Action::FansAuto => Feature::FanControl,
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::SetPowerLimit { limit } => write!(f, "power limit {limit}"),
            Action::SetClockOffset { domain, offset } => write!(f, "{domain} offset {offset}"),
            Action::SetFanDuty { duty } => write!(f, "fans fixed {duty}"),
            Action::FansAuto => f.write_str("fans auto"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    /// The driver accepted the write but reads back a different value.
    NotEnforced {
        expected: String,
        actual: String,
    },
    /// Refused before writing: out of range, unsupported, or against policy.
    Rejected {
        reason: String,
    },
    /// Refused by the compatibility matrix.
    Blocked {
        reason: String,
    },
    /// The driver returned an error.
    Failed {
        error: String,
    },
}

impl Outcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Outcome::Applied)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemReport {
    pub item: String,
    pub feature: Feature,
    pub outcome: Outcome,
}

/// What [`plan`] decided.
#[derive(Debug, Clone)]
pub struct Plan {
    pub actions: Vec<Action>,
    /// Items refused during planning; reported alongside executed ones.
    pub rejected: Vec<ItemReport>,
    /// A curve for the service's control loop to run.
    pub fan_curve: Option<(FanCurve, FanTuning)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub device: crate::device::DeviceId,
    pub profile: String,
    pub items: Vec<ItemReport>,
}

impl ApplyReport {
    pub fn all_applied(&self) -> bool {
        self.items.iter().all(|i| i.outcome.is_success())
    }
}

fn reject(item: String, feature: Feature, reason: impl Into<String>) -> ItemReport {
    ItemReport { item, feature, outcome: Outcome::Rejected { reason: reason.into() } }
}

/// Resolve `profile` against `caps` and `policy`. Pure; writes nothing.
pub fn plan(profile: &Profile, caps: &Capabilities, policy: &SafetyPolicy) -> Plan {
    let mut actions = Vec::new();
    let mut rejected = Vec::new();
    let mut fan_curve = None;

    if let Some(power) = &profile.power {
        let label = format!("power limit {power:?}");
        match &caps.power_limit {
            None => rejected.push(reject(
                label,
                Feature::PowerLimit,
                "power limit is not adjustable on this device",
            )),
            Some(pl) => {
                let limit = power.resolve(pl);
                let policy_max = policy
                    .max_power_percent
                    .map(|p| Milliwatts(((pl.default.0 as f64) * (p as f64) / 100.0).round() as u32));
                if limit < pl.min || limit > pl.max {
                    rejected.push(reject(
                        format!("power limit {limit}"),
                        Feature::PowerLimit,
                        format!("{limit} is outside the driver range {}..={}", pl.min, pl.max),
                    ));
                } else if let Some(max) = policy_max.filter(|m| limit > *m) {
                    rejected.push(reject(
                        format!("power limit {limit}"),
                        Feature::PowerLimit,
                        format!("{limit} exceeds the policy cap {max}"),
                    ));
                } else {
                    actions.push(Action::SetPowerLimit { limit });
                }
            }
        }
    }

    for (domain, requested, policy_cap) in [
        (ClockDomain::Core, profile.clocks.core_offset_mhz, policy.max_core_offset_mhz),
        (ClockDomain::Memory, profile.clocks.memory_offset_mhz, policy.max_memory_offset_mhz),
    ] {
        let Some(mhz) = requested else { continue };
        let offset = MhzOffset(mhz);
        let label = format!("{domain} offset {offset}");
        match caps.offset(domain) {
            None => rejected.push(reject(
                label,
                Feature::ClockOffset,
                format!("{domain} offset is not adjustable on this device"),
            )),
            Some(range) if !range.contains(offset) => rejected.push(reject(
                label,
                Feature::ClockOffset,
                format!("{offset} is outside the driver range {}..={}", range.min, range.max),
            )),
            Some(_) if policy_cap.is_some_and(|cap| mhz.unsigned_abs() > cap.unsigned_abs()) => {
                rejected.push(reject(label, Feature::ClockOffset, "exceeds the policy cap"));
            }
            Some(_) => actions.push(Action::SetClockOffset { domain, offset }),
        }
    }

    match &profile.fan {
        None => {}
        Some(FanSetting::Auto) => match caps.fans {
            Some(_) => actions.push(Action::FansAuto),
            None => rejected.push(reject(
                "fans auto".into(),
                Feature::FanControl,
                "fan control is not available on this device",
            )),
        },
        Some(FanSetting::Fixed { duty }) => match caps.fans {
            Some(fc) => actions.push(Action::SetFanDuty { duty: fc.clamp(*duty) }),
            None => rejected.push(reject(
                format!("fans fixed {duty}"),
                Feature::FanControl,
                "fan control is not available on this device",
            )),
        },
        Some(setting @ FanSetting::Curve { tuning, .. }) => match (caps.fans, setting.curve()) {
            (None, _) => rejected.push(reject(
                "fan curve".into(),
                Feature::FanControl,
                "fan control is not available on this device",
            )),
            (_, Some(Err(e))) => {
                rejected.push(reject("fan curve".into(), Feature::FanControl, e.to_string()))
            }
            (Some(_), Some(Ok(curve))) => {
                let mut tuning = tuning.clone();
                // Never let a user curve run past the point the firmware starts throttling.
                if let Some(slow) = caps.slowdown_temp
                    && tuning.critical_temp.0 > slow.0
                {
                    tuning.critical_temp = slow;
                }
                fan_curve = Some((curve, tuning));
            }
            (Some(_), None) => {}
        },
    }

    Plan { actions, rejected, fan_curve }
}

/// Check the matrix for `feature` on this device.
pub fn compat_gate(
    info: &DeviceInfo,
    feature: Feature,
    matrix: &CompatMatrix,
    policy: &SafetyPolicy,
) -> Result<(), String> {
    match matrix.lookup(info.vendor, feature, info.driver_version.as_deref(), info.architecture.as_deref()) {
        Verdict::Verified { .. } => Ok(()),
        Verdict::Broken { note } => Err(format!(
            "{feature} is known broken on driver {}: {note}",
            info.driver_version.as_deref().unwrap_or("unknown")
        )),
        Verdict::Unverified if policy.allow_unverified => Ok(()),
        Verdict::Unverified => Err(format!(
            "{feature} has not been verified on {} driver {} ({}). Re-run with --allow-unverified \
             to try it, then submit a hardware report",
            info.vendor,
            info.driver_version.as_deref().unwrap_or("unknown"),
            info.architecture.as_deref().unwrap_or("unknown architecture"),
        )),
    }
}

/// Write one action and read it back. Public for `sol selftest`, which exercises each control
/// to produce compatibility evidence. Everything else should go through [`execute`].
pub fn write_and_verify(device: &mut dyn GpuDevice, action: &Action) -> Outcome {
    let result = match action {
        Action::SetPowerLimit { limit } => device.set_power_limit(*limit),
        Action::SetClockOffset { domain, offset } => device.set_clock_offset(*domain, *offset),
        Action::SetFanDuty { duty } => device.set_fan_duty(*duty),
        Action::FansAuto => device.set_fans_auto(),
    };
    if let Err(e) = result {
        return Outcome::Failed { error: e.to_string() };
    }

    match action {
        Action::SetPowerLimit { limit } => match device.tuning_state().map(|s| s.power_limit) {
            Ok(Some(actual)) if actual == *limit => Outcome::Applied,
            Ok(Some(actual)) => {
                Outcome::NotEnforced { expected: limit.to_string(), actual: actual.to_string() }
            }
            Ok(None) | Err(DeviceError::NotSupported) => unreadable(),
            Err(e) => Outcome::Failed { error: format!("read-back failed: {e}") },
        },
        Action::SetClockOffset { domain, offset } => match device.tuning_state().map(|s| s.offset(*domain)) {
            Ok(Some(actual)) if actual == *offset => Outcome::Applied,
            Ok(Some(actual)) => {
                Outcome::NotEnforced { expected: offset.to_string(), actual: actual.to_string() }
            }
            Ok(None) | Err(DeviceError::NotSupported) => unreadable(),
            Err(e) => Outcome::Failed { error: format!("read-back failed: {e}") },
        },
        Action::SetFanDuty { duty } => match device.telemetry() {
            Ok(t) => {
                let off: Vec<String> = t
                    .fans
                    .iter()
                    .filter_map(|f| f.duty.filter(|d| (d.get() - duty.get()).abs() > FAN_TOLERANCE))
                    .map(|d| d.to_string())
                    .collect();
                if t.fans.iter().all(|f| f.duty.is_none()) {
                    unreadable()
                } else if off.is_empty() {
                    Outcome::Applied
                } else {
                    Outcome::NotEnforced { expected: duty.to_string(), actual: off.join(", ") }
                }
            }
            Err(e) => Outcome::Failed { error: format!("read-back failed: {e}") },
        },
        // Firmware-auto has no target to compare against.
        Action::FansAuto => Outcome::Applied,
    }
}

fn unreadable() -> Outcome {
    Outcome::NotEnforced {
        expected: "a readable value".into(),
        actual: "the driver does not report this setting, so the write can't be verified".into(),
    }
}

/// Apply a plan to a device. Every action is gated, written, and read back.
pub fn execute(
    device: &mut dyn GpuDevice,
    profile_name: &str,
    plan: &Plan,
    matrix: &CompatMatrix,
    policy: &SafetyPolicy,
) -> ApplyReport {
    let info = device.info().clone();
    execute_with(device, profile_name, plan, |feature| compat_gate(&info, feature, matrix, policy))
}

/// Everything back to stock: default power limit, zero offsets, firmware fans.
pub fn reset_plan(caps: &Capabilities) -> Plan {
    let mut actions = Vec::new();
    if let Some(pl) = caps.power_limit {
        actions.push(Action::SetPowerLimit { limit: pl.default });
    }
    for domain in [ClockDomain::Core, ClockDomain::Memory] {
        if caps.offset(domain).is_some() {
            actions.push(Action::SetClockOffset { domain, offset: MhzOffset(0) });
        }
    }
    if caps.fans.is_some() {
        actions.push(Action::FansAuto);
    }
    Plan { actions, rejected: Vec::new(), fan_curve: None }
}

/// Apply [`reset_plan`]. Not gated by the matrix: returning to stock must always be possible,
/// even on a driver where tuning is known broken.
pub fn reset(device: &mut dyn GpuDevice, caps: &Capabilities) -> ApplyReport {
    execute_with(device, "stock", &reset_plan(caps), |_| Ok(()))
}

fn execute_with(
    device: &mut dyn GpuDevice,
    profile_name: &str,
    plan: &Plan,
    gate: impl Fn(Feature) -> Result<(), String>,
) -> ApplyReport {
    let info = device.info().clone();
    let mut items = plan.rejected.clone();
    for action in &plan.actions {
        let feature = action.feature();
        let outcome = match gate(feature) {
            Err(reason) => Outcome::Blocked { reason },
            Ok(()) => write_and_verify(device, action),
        };
        tracing::info!(device = %info.id, %action, ?outcome, "apply");
        items.push(ItemReport { item: action.to_string(), feature, outcome });
    }
    if plan.fan_curve.is_some() {
        let outcome = match gate(Feature::FanControl) {
            Err(reason) => Outcome::Blocked { reason },
            // The service starts the curve; it reports failures from the loop.
            Ok(()) => Outcome::Applied,
        };
        items.push(ItemReport { item: "fan curve".into(), feature: Feature::FanControl, outcome });
    }
    ApplyReport { device: info.id, profile: profile_name.to_string(), items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::sim::{SimConfig, SimulatedGpu};

    fn allow() -> SafetyPolicy {
        SafetyPolicy { allow_unverified: true, ..SafetyPolicy::default() }
    }

    fn profile(body: &str) -> Profile {
        Profile::from_toml(&format!("schema = 1\nname = \"t\"\n{body}")).unwrap()
    }

    fn run(gpu: &mut SimulatedGpu, p: &Profile, policy: &SafetyPolicy) -> ApplyReport {
        let caps = gpu.capabilities().unwrap();
        let plan = plan(p, &caps, policy);
        execute(gpu, &p.name, &plan, &CompatMatrix::default(), policy)
    }

    #[test]
    fn applies_and_verifies_a_full_profile() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        let p = profile(
            "[power]\npercent = 90\n[clocks]\ncore_offset_mhz = 100\nmemory_offset_mhz = 500\n[fan]\nmode = \"fixed\"\nduty = 60\n",
        );
        let report = run(&mut gpu, &p, &allow());
        assert!(report.all_applied(), "{report:#?}");
        let state = gpu.tuning_state().unwrap();
        assert_eq!(state.core_offset, Some(MhzOffset(100)));
        assert_eq!(state.memory_offset, Some(MhzOffset(500)));
        assert_eq!(state.power_limit, Some(Milliwatts(288_000)));
    }

    #[test]
    fn out_of_range_values_are_rejected_before_writing() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        let p = profile("[power]\npercent = 150\n[clocks]\ncore_offset_mhz = 5000\n");
        let report = run(&mut gpu, &p, &allow());
        assert_eq!(report.items.len(), 2);
        assert!(report.items.iter().all(|i| matches!(i.outcome, Outcome::Rejected { .. })));
        assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(0)));
    }

    #[test]
    fn policy_caps_apply_inside_the_driver_range() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        let policy =
            SafetyPolicy { max_power_percent: Some(100.0), max_core_offset_mhz: Some(150), ..allow() };
        let p = profile("[power]\npercent = 105\n[clocks]\ncore_offset_mhz = -200\n");
        let report = run(&mut gpu, &p, &policy);
        assert!(report.items.iter().all(|i| matches!(i.outcome, Outcome::Rejected { .. })), "{report:#?}");
    }

    #[test]
    fn unverified_drivers_are_blocked_by_default() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        let p = profile("[clocks]\ncore_offset_mhz = 50\n");
        let report = run(&mut gpu, &p, &SafetyPolicy::default());
        assert!(matches!(report.items[0].outcome, Outcome::Blocked { .. }));
        assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(0)));
    }

    #[test]
    fn known_broken_is_blocked_even_when_unverified_is_allowed() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        let matrix = CompatMatrix::from_toml(
            "[[entry]]\nvendor = \"simulated\"\nfeature = \"clock_offset\"\nstatus = \"broken\"\nnote = \"sim\"\nevidence = \"test\"\n",
        )
        .unwrap();
        let p = profile("[clocks]\ncore_offset_mhz = 50\n");
        let caps = gpu.capabilities().unwrap();
        let report = execute(&mut gpu, "t", &plan(&p, &caps, &allow()), &matrix, &allow());
        assert!(matches!(report.items[0].outcome, Outcome::Blocked { .. }));
    }

    #[test]
    fn accepted_but_ignored_writes_are_not_enforced() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        gpu.with_faults(|f| {
            f.ignore_offset_writes = true;
            f.ignore_fan_writes = true;
        });
        let p = profile("[clocks]\ncore_offset_mhz = 75\n[fan]\nmode = \"fixed\"\nduty = 80\n");
        let report = run(&mut gpu, &p, &allow());
        assert!(report.items.iter().all(|i| matches!(i.outcome, Outcome::NotEnforced { .. })), "{report:#?}");
    }

    #[test]
    fn driver_errors_are_reported_per_item() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        gpu.with_faults(|f| f.permission_denied = true);
        let p = profile("[power]\npercent = 95\n");
        let report = run(&mut gpu, &p, &allow());
        assert!(
            matches!(&report.items[0].outcome, Outcome::Failed { error } if error.contains("permission"))
        );
    }

    #[test]
    fn fixed_fan_is_raised_to_driver_minimum() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        let p = profile("[fan]\nmode = \"fixed\"\nduty = 10\n");
        let caps = gpu.capabilities().unwrap();
        let plan = plan(&p, &caps, &allow());
        assert_eq!(plan.actions, vec![Action::SetFanDuty { duty: caps.fans.unwrap().min_duty }]);
        assert!(execute(&mut gpu, "t", &plan, &CompatMatrix::default(), &allow()).all_applied());
    }

    #[test]
    fn reset_restores_stock_and_verifies_it() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        let p = profile(
            "[power]\npercent = 80\n[clocks]\ncore_offset_mhz = 120\n[fan]\nmode = \"fixed\"\nduty = 70\n",
        );
        assert!(run(&mut gpu, &p, &allow()).all_applied());
        let caps = gpu.capabilities().unwrap();
        let report = reset(&mut gpu, &caps);
        assert!(report.all_applied(), "{report:#?}");
        let s = gpu.tuning_state().unwrap();
        assert_eq!(s.power_limit, Some(caps.power_limit.unwrap().default));
        assert_eq!(s.core_offset, Some(MhzOffset(0)));
        assert!(gpu.fans_on_firmware());
    }

    #[test]
    fn curve_critical_temp_is_capped_at_firmware_slowdown() {
        let gpu = SimulatedGpu::new(SimConfig::default());
        let caps = gpu.capabilities().unwrap();
        let p = profile(
            "[fan]\nmode = \"curve\"\npoints = [[30, 30], [80, 100]]\n[fan.tuning]\ncritical_temp = 105\n",
        );
        let plan = plan(&p, &caps, &allow());
        let (_, tuning) = plan.fan_curve.unwrap();
        assert_eq!(tuning.critical_temp, caps.slowdown_temp.unwrap());
    }
}
