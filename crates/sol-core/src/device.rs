//! What a device is, what it can do, and what it is doing right now.
//!
//! Every reading is an `Option`: a sensor the driver doesn't expose is reported as missing,
//! never as zero. Afterburner-style tools that print `0 °C` for an absent sensor teach users
//! to distrust every number on the screen.

use crate::units::{Celsius, Mhz, MhzOffset, Milliwatts, Percent, Rpm};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Simulated,
}

impl fmt::Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Simulated => "Simulated",
        })
    }
}

/// A stable identifier: `<vendor>:<vendor-specific id>`, e.g. `nvidia:GPU-1a2b...`.
/// Stable across reboots and slot changes, unlike an enumeration index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub vendor: Vendor,
    pub name: String,
    /// Vendor architecture name, e.g. `Ampere`. Used to key the compatibility matrix.
    pub architecture: Option<String>,
    pub driver_version: Option<String>,
    pub vbios_version: Option<String>,
    pub pci_bus_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockDomain {
    Core,
    Memory,
}

impl fmt::Display for ClockDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ClockDomain::Core => "core",
            ClockDomain::Memory => "memory",
        })
    }
}

/// Driver-reported power-limit range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PowerLimitCaps {
    pub min: Milliwatts,
    pub max: Milliwatts,
    pub default: Milliwatts,
}

/// Driver-reported clock-offset range for one domain.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OffsetCaps {
    pub min: MhzOffset,
    pub max: MhzOffset,
}

impl OffsetCaps {
    pub fn contains(&self, offset: MhzOffset) -> bool {
        self.min <= offset && offset <= self.max
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FanCaps {
    pub count: u32,
    /// Lowest duty the driver accepts. Many cards refuse anything under ~30 %.
    pub min_duty: Percent,
    pub max_duty: Percent,
}

impl FanCaps {
    /// Raises a requested duty to the driver minimum. Raising is the only safe direction.
    pub fn clamp(&self, duty: Percent) -> Percent {
        Percent::clamped(duty.get().clamp(self.min_duty.get(), self.max_duty.get()))
    }
}

/// What this device, on this driver, says it lets us control. `None` means not exposed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub power_limit: Option<PowerLimitCaps>,
    pub core_offset: Option<OffsetCaps>,
    pub memory_offset: Option<OffsetCaps>,
    pub fans: Option<FanCaps>,
    /// Temperature at which the firmware starts slowing the GPU down.
    pub slowdown_temp: Option<Celsius>,
    /// Temperature at which the firmware shuts the GPU off.
    pub shutdown_temp: Option<Celsius>,
}

impl Capabilities {
    pub fn offset(&self, domain: ClockDomain) -> Option<&OffsetCaps> {
        match domain {
            ClockDomain::Core => self.core_offset.as_ref(),
            ClockDomain::Memory => self.memory_offset.as_ref(),
        }
    }
}

/// Why the GPU isn't running faster right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerfLimiter {
    Idle,
    Power,
    Thermal,
    HwSlowdown,
    PowerBrake,
    Applications,
    Display,
    SyncBoost,
}

impl fmt::Display for PerfLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PerfLimiter::Idle => "idle",
            PerfLimiter::Power => "power",
            PerfLimiter::Thermal => "thermal",
            PerfLimiter::HwSlowdown => "hw-slowdown",
            PerfLimiter::PowerBrake => "power-brake",
            PerfLimiter::Applications => "app-clocks",
            PerfLimiter::Display => "display",
            PerfLimiter::SyncBoost => "sync-boost",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanReading {
    pub index: u32,
    pub duty: Option<Percent>,
    pub rpm: Option<Rpm>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Telemetry {
    pub gpu_temp: Option<Celsius>,
    pub core_clock: Option<Mhz>,
    pub memory_clock: Option<Mhz>,
    pub power_draw: Option<Milliwatts>,
    pub power_limit: Option<Milliwatts>,
    pub gpu_utilization: Option<Percent>,
    pub memory_utilization: Option<Percent>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub fans: Vec<FanReading>,
    pub limiters: Vec<PerfLimiter>,
    pub pstate: Option<u8>,
}

/// Read-back of the values we can set. Used to check a write actually took effect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuningState {
    pub power_limit: Option<Milliwatts>,
    pub core_offset: Option<MhzOffset>,
    pub memory_offset: Option<MhzOffset>,
}

impl TuningState {
    pub fn offset(&self, domain: ClockDomain) -> Option<MhzOffset> {
        match domain {
            ClockDomain::Core => self.core_offset,
            ClockDomain::Memory => self.memory_offset,
        }
    }
}
