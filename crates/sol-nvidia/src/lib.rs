//! NVIDIA backend over NVML (`nvml.dll`, installed with every NVIDIA driver).
//!
//! NVML is NVIDIA's documented, supported management API. It covers telemetry, power limits,
//! clock offsets (`nvmlDeviceSetClockOffsets`, R555+) and fan duty. It does **not** expose the
//! per-point V/F curve or voltage control; those need undocumented NVAPI calls. See
//! `docs/vendor-apis.md` for what that would take and why it isn't here yet.
//!
//! The backend stays thin: it translates calls and errors. Range checks, the compatibility
//! matrix and read-back verification all live in `sol_core::tuner`.

use nvml_wrapper::Nvml;
use nvml_wrapper::bitmasks::device::ThrottleReasons;
use nvml_wrapper::enum_wrappers::device::{Clock, PerformanceState, TemperatureSensor, TemperatureThreshold};
use nvml_wrapper::error::NvmlError;
use sol_core::backend::{Backend, BackendError, DeviceError, DeviceResult, GpuDevice};
use sol_core::device::{
    Capabilities, ClockDomain, DeviceId, DeviceInfo, FanCaps, FanReading, OffsetCaps, PerfLimiter,
    PowerLimitCaps, Telemetry, TuningState, Vendor,
};
use sol_core::units::{Celsius, Mhz, MhzOffset, Milliwatts, Percent, Rpm};
use std::sync::Arc;

#[derive(Debug)]
pub struct NvidiaBackend {
    nvml: Arc<Nvml>,
}

impl NvidiaBackend {
    pub fn open() -> Result<Self, BackendError> {
        match Nvml::init() {
            Ok(nvml) => Ok(NvidiaBackend { nvml: Arc::new(nvml) }),
            Err(NvmlError::LibloadingError(e)) => {
                Err(BackendError::NotPresent(format!("NVML not found (no NVIDIA driver?): {e}")))
            }
            Err(NvmlError::DriverNotLoaded) => {
                Err(BackendError::NotPresent("NVIDIA driver is not loaded".into()))
            }
            Err(e) => Err(BackendError::Init(format!("NVML init failed: {e}"))),
        }
    }

    pub fn nvml_version(&self) -> Option<String> {
        self.nvml.sys_nvml_version().ok()
    }
}

impl Backend for NvidiaBackend {
    fn name(&self) -> &'static str {
        "nvidia-nvml"
    }

    fn devices(&mut self) -> Result<Vec<Box<dyn GpuDevice>>, BackendError> {
        let count = self.nvml.device_count().map_err(|e| BackendError::Init(e.to_string()))?;
        let driver = self.nvml.sys_driver_version().ok();
        let mut out: Vec<Box<dyn GpuDevice>> = Vec::new();
        for index in 0..count {
            let d = match self.nvml.device_by_index(index) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(index, error = %e, "skipping NVIDIA device");
                    continue;
                }
            };
            let uuid = match d.uuid() {
                Ok(u) => u,
                Err(e) => {
                    tracing::warn!(index, error = %e, "skipping NVIDIA device without a UUID");
                    continue;
                }
            };
            let info = DeviceInfo {
                id: DeviceId(format!("nvidia:{uuid}")),
                vendor: Vendor::Nvidia,
                name: d.name().unwrap_or_else(|_| format!("NVIDIA GPU {index}")),
                architecture: d.architecture().ok().map(|a| a.to_string()),
                driver_version: driver.clone(),
                vbios_version: d.vbios_version().ok(),
                pci_bus_id: d.pci_info().ok().map(|p| p.bus_id),
            };
            out.push(Box::new(NvidiaDevice { nvml: Arc::clone(&self.nvml), uuid, info }));
        }
        Ok(out)
    }
}

#[derive(Debug)]
pub struct NvidiaDevice {
    nvml: Arc<Nvml>,
    /// Handles are re-resolved by UUID on every call: an index can change across a driver
    /// reset, a UUID can't.
    uuid: String,
    info: DeviceInfo,
}

fn map_err(e: NvmlError) -> DeviceError {
    match e {
        NvmlError::NotSupported | NvmlError::FailedToLoadSymbol(_) => DeviceError::NotSupported,
        NvmlError::NoPermission => DeviceError::NoPermission,
        NvmlError::InvalidArg => DeviceError::InvalidValue("NVML rejected the argument".into()),
        NvmlError::GpuLost | NvmlError::ResetRequired => DeviceError::Lost,
        other => DeviceError::Other(other.to_string()),
    }
}

/// Telemetry read: absent sensors become `None`; a lost GPU is still an error.
fn opt<T>(r: Result<T, NvmlError>) -> DeviceResult<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) => match map_err(e) {
            DeviceError::Lost => Err(DeviceError::Lost),
            _ => Ok(None),
        },
    }
}

fn pstate_number(p: PerformanceState) -> Option<u8> {
    use PerformanceState::*;
    Some(match p {
        Zero => 0,
        One => 1,
        Two => 2,
        Three => 3,
        Four => 4,
        Five => 5,
        Six => 6,
        Seven => 7,
        Eight => 8,
        Nine => 9,
        Ten => 10,
        Eleven => 11,
        Twelve => 12,
        Thirteen => 13,
        Fourteen => 14,
        Fifteen => 15,
        Unknown => return None,
    })
}

fn limiters(r: ThrottleReasons) -> Vec<PerfLimiter> {
    let map = [
        (ThrottleReasons::GPU_IDLE, PerfLimiter::Idle),
        (ThrottleReasons::SW_POWER_CAP, PerfLimiter::Power),
        (ThrottleReasons::SW_THERMAL_SLOWDOWN | ThrottleReasons::HW_THERMAL_SLOWDOWN, PerfLimiter::Thermal),
        (ThrottleReasons::HW_SLOWDOWN, PerfLimiter::HwSlowdown),
        (ThrottleReasons::HW_POWER_BRAKE_SLOWDOWN, PerfLimiter::PowerBrake),
        (ThrottleReasons::APPLICATIONS_CLOCKS_SETTING, PerfLimiter::Applications),
        (ThrottleReasons::DISPLAY_CLOCK_SETTING, PerfLimiter::Display),
        (ThrottleReasons::SYNC_BOOST, PerfLimiter::SyncBoost),
    ];
    map.into_iter().filter(|(bits, _)| r.intersects(*bits)).map(|(_, l)| l).collect()
}

fn nvml_clock(domain: ClockDomain) -> Clock {
    match domain {
        ClockDomain::Core => Clock::Graphics,
        ClockDomain::Memory => Clock::Memory,
    }
}

impl NvidiaDevice {
    fn with<T>(&self, f: impl FnOnce(&nvml_wrapper::Device<'_>) -> DeviceResult<T>) -> DeviceResult<T> {
        let d = self.nvml.device_by_uuid(self.uuid.as_str()).map_err(map_err)?;
        f(&d)
    }

    fn with_mut<T>(
        &self,
        f: impl FnOnce(&mut nvml_wrapper::Device<'_>) -> DeviceResult<T>,
    ) -> DeviceResult<T> {
        let mut d = self.nvml.device_by_uuid(self.uuid.as_str()).map_err(map_err)?;
        f(&mut d)
    }

    fn offset_caps(d: &nvml_wrapper::Device<'_>, domain: ClockDomain) -> DeviceResult<Option<OffsetCaps>> {
        Ok(opt(d.clock_offset(nvml_clock(domain), PerformanceState::Zero))?.map(|o| OffsetCaps {
            min: MhzOffset(o.min_clock_offset_mhz),
            max: MhzOffset(o.max_clock_offset_mhz),
        }))
    }
}

impl GpuDevice for NvidiaDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn capabilities(&self) -> DeviceResult<Capabilities> {
        self.with(|d| {
            let power_limit = match (
                opt(d.power_management_limit_constraints())?,
                opt(d.power_management_limit_default())?,
            ) {
                (Some(c), Some(default)) => Some(PowerLimitCaps {
                    min: Milliwatts(c.min_limit),
                    max: Milliwatts(c.max_limit),
                    default: Milliwatts(default),
                }),
                _ => None,
            };
            let fans = match opt(d.num_fans())? {
                Some(count) if count > 0 => {
                    let (min, max) = opt(d.min_max_fan_speed())?.unwrap_or((0, 100));
                    Some(FanCaps {
                        count,
                        min_duty: Percent::clamped(min as f32),
                        max_duty: Percent::clamped(max as f32),
                    })
                }
                _ => None,
            };
            Ok(Capabilities {
                power_limit,
                core_offset: Self::offset_caps(d, ClockDomain::Core)?,
                memory_offset: Self::offset_caps(d, ClockDomain::Memory)?,
                fans,
                slowdown_temp: opt(d.temperature_threshold(TemperatureThreshold::Slowdown))?
                    .map(|t| Celsius(t as f32)),
                shutdown_temp: opt(d.temperature_threshold(TemperatureThreshold::Shutdown))?
                    .map(|t| Celsius(t as f32)),
            })
        })
    }

    fn telemetry(&self) -> DeviceResult<Telemetry> {
        self.with(|d| {
            let util = opt(d.utilization_rates())?;
            let mem = opt(d.memory_info())?;
            let fan_count = opt(d.num_fans())?.unwrap_or(0);
            let mut fans = Vec::with_capacity(fan_count as usize);
            for index in 0..fan_count {
                fans.push(FanReading {
                    index,
                    duty: opt(d.fan_speed(index))?.map(|v| Percent::clamped(v as f32)),
                    rpm: opt(d.fan_speed_rpm(index))?.map(Rpm),
                });
            }
            #[allow(deprecated)]
            let reasons = opt(d.current_throttle_reasons())?;
            Ok(Telemetry {
                gpu_temp: opt(d.temperature(TemperatureSensor::Gpu))?.map(|t| Celsius(t as f32)),
                core_clock: opt(d.clock_info(Clock::Graphics))?.map(Mhz),
                memory_clock: opt(d.clock_info(Clock::Memory))?.map(Mhz),
                power_draw: opt(d.power_usage())?.map(Milliwatts),
                power_limit: opt(d.enforced_power_limit())?.map(Milliwatts),
                gpu_utilization: util.as_ref().map(|u| Percent::clamped(u.gpu as f32)),
                memory_utilization: util.as_ref().map(|u| Percent::clamped(u.memory as f32)),
                vram_used_bytes: mem.as_ref().map(|m| m.used),
                vram_total_bytes: mem.as_ref().map(|m| m.total),
                fans,
                limiters: reasons.map(limiters).unwrap_or_default(),
                pstate: opt(d.performance_state())?.and_then(pstate_number),
            })
        })
    }

    fn tuning_state(&self) -> DeviceResult<TuningState> {
        self.with(|d| {
            let offset = |domain| -> DeviceResult<Option<MhzOffset>> {
                Ok(opt(d.clock_offset(nvml_clock(domain), PerformanceState::Zero))?
                    .map(|o| MhzOffset(o.clock_offset_mhz)))
            };
            Ok(TuningState {
                power_limit: opt(d.power_management_limit())?.map(Milliwatts),
                core_offset: offset(ClockDomain::Core)?,
                memory_offset: offset(ClockDomain::Memory)?,
            })
        })
    }

    fn set_power_limit(&mut self, limit: Milliwatts) -> DeviceResult<()> {
        self.with_mut(|d| d.set_power_management_limit(limit.0).map_err(map_err))
    }

    fn set_clock_offset(&mut self, domain: ClockDomain, offset: MhzOffset) -> DeviceResult<()> {
        self.with_mut(|d| {
            d.set_clock_offset(nvml_clock(domain), PerformanceState::Zero, offset.0).map_err(map_err)
        })
    }

    fn set_fan_duty(&mut self, duty: Percent) -> DeviceResult<()> {
        self.with_mut(|d| {
            let count = d.num_fans().map_err(map_err)?;
            let duty = duty.get().round() as u32;
            for fan in 0..count {
                d.set_fan_speed(fan, duty).map_err(map_err)?;
            }
            Ok(())
        })
    }

    fn set_fans_auto(&mut self) -> DeviceResult<()> {
        self.with_mut(|d| {
            let count = d.num_fans().map_err(map_err)?;
            // Try every fan even if one fails: leaving some fans under software control is
            // worse than reporting a partial failure.
            let mut first_err = None;
            for fan in 0..count {
                if let Err(e) = d.set_default_fan_speed(fan) {
                    first_err.get_or_insert(map_err(e));
                }
            }
            first_err.map_or(Ok(()), Err)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_throttle_reasons() {
        let l = limiters(ThrottleReasons::SW_POWER_CAP | ThrottleReasons::HW_THERMAL_SLOWDOWN);
        assert_eq!(l, vec![PerfLimiter::Power, PerfLimiter::Thermal]);
        assert!(limiters(ThrottleReasons::NONE).is_empty());
    }

    #[test]
    fn maps_errors() {
        assert_eq!(map_err(NvmlError::NotSupported), DeviceError::NotSupported);
        assert_eq!(map_err(NvmlError::NoPermission), DeviceError::NoPermission);
        assert_eq!(map_err(NvmlError::GpuLost), DeviceError::Lost);
        assert!(matches!(opt::<u32>(Err(NvmlError::NotSupported)), Ok(None)));
        assert!(matches!(opt::<u32>(Err(NvmlError::GpuLost)), Err(DeviceError::Lost)));
    }

    /// Read-only check against real hardware. Run with `cargo test -p sol-nvidia -- --ignored`.
    /// Never writes anything.
    #[test]
    #[ignore = "needs an NVIDIA GPU"]
    fn live_read_only_probe() {
        let mut backend = NvidiaBackend::open().expect("NVML should load on a machine with an NVIDIA GPU");
        let devices = backend.devices().expect("enumerate");
        assert!(!devices.is_empty(), "no NVIDIA devices found");
        for d in &devices {
            let info = d.info();
            assert!(info.id.0.starts_with("nvidia:GPU-"), "{:?}", info.id);
            assert!(info.driver_version.is_some());
            let caps = d.capabilities().expect("capabilities");
            let t = d.telemetry().expect("telemetry");
            let s = d.tuning_state().expect("tuning state");
            let temp = t.gpu_temp.expect("every NVIDIA GPU reports a die temperature").0;
            assert!((5.0..110.0).contains(&temp), "implausible temperature {temp}");
            if let (Some(pl), Some(cur)) = (caps.power_limit, s.power_limit) {
                assert!(pl.min <= cur && cur <= pl.max, "{pl:?} vs {cur:?}");
            }
            eprintln!("{info:#?}\n{caps:#?}\n{t:#?}\n{s:#?}");
        }
    }
}
