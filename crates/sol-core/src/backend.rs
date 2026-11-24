//! The seam between vendor code and everything else.

use crate::device::{Capabilities, ClockDomain, DeviceInfo, Telemetry, TuningState};
use crate::units::{MhzOffset, Milliwatts, Percent};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DeviceError {
    #[error("not supported by this device or driver")]
    NotSupported,
    #[error("permission denied (writes need the service running as administrator)")]
    NoPermission,
    #[error("value rejected by the driver: {0}")]
    InvalidValue(String),
    #[error("device lost (driver reset or GPU fell off the bus)")]
    Lost,
    #[error("{0}")]
    Other(String),
}

pub type DeviceResult<T> = Result<T, DeviceError>;

/// One GPU. Implementations must be thin: translate calls and errors, nothing else.
/// Validation, policy and verification belong to [`crate::tuner`], not to backends.
pub trait GpuDevice: Send + std::fmt::Debug {
    fn info(&self) -> &DeviceInfo;
    fn capabilities(&self) -> DeviceResult<Capabilities>;
    fn telemetry(&self) -> DeviceResult<Telemetry>;
    fn tuning_state(&self) -> DeviceResult<TuningState>;

    fn set_power_limit(&mut self, limit: Milliwatts) -> DeviceResult<()>;
    fn set_clock_offset(&mut self, domain: ClockDomain, offset: MhzOffset) -> DeviceResult<()>;
    /// Sets every fan to `duty`.
    fn set_fan_duty(&mut self, duty: Percent) -> DeviceResult<()>;
    /// Hands every fan back to the firmware's own curve.
    fn set_fans_auto(&mut self) -> DeviceResult<()>;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BackendError {
    /// The vendor's runtime isn't installed: no such GPU, or no driver.
    #[error("{0}")]
    NotPresent(String),
    /// The runtime is present but this backend can't use it yet.
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Init(String),
}

/// A vendor runtime (NVML, ADLX, IGCL, or the simulator).
pub trait Backend: Send + std::fmt::Debug {
    fn name(&self) -> &'static str;
    fn devices(&mut self) -> Result<Vec<Box<dyn GpuDevice>>, BackendError>;
}
