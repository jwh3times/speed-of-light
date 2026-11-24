//! Vendor-neutral core of Speed of Light.
//!
//! Nothing in this crate talks to hardware. Vendor backends (`sol-nvidia`, `sol-amd`,
//! `sol-intel`) implement [`backend::GpuDevice`]; everything that decides *whether* and *how*
//! to write to a GPU lives here so it can be tested without one:
//!
//! - [`tuner`] turns a [`profile::Profile`] into a plan, checks it against the device's
//!   reported limits and the [`compat`] matrix, applies it, and reads every value back.
//! - [`fan`] is the fan-curve engine (hysteresis, ramp limits, sensor-failure fallback).
//! - [`crashguard`] stops a profile that crashed the machine from being re-applied at boot.
//! - [`sim`] is a simulated GPU with a thermal model and fault injection for tests.

pub mod backend;
pub mod compat;
pub mod crashguard;
pub mod device;
pub mod fan;
pub mod profile;
pub mod sim;
pub mod tuner;
pub mod units;

pub use backend::{Backend, BackendError, DeviceError, DeviceResult, GpuDevice};
pub use device::{
    Capabilities, ClockDomain, DeviceId, DeviceInfo, FanCaps, FanReading, OffsetCaps, PerfLimiter,
    PowerLimitCaps, Telemetry, TuningState, Vendor,
};
pub use units::{Celsius, Mhz, MhzOffset, Milliwatts, Percent, Rpm};
