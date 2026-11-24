//! A simulated GPU: a first-order thermal model, firmware-style auto fan, power capping, and
//! fault injection. It's what tests, `sol-service --simulate` and UI development run against.
//!
//! The model is deliberately simple. It only has to behave in the right *direction*
//! (more fan = cooler, more power = hotter, the power limit caps draw) so control-loop tests
//! can assert that, for example, a fan curve actually stops a heating GPU from reaching
//! slowdown.
//!
//! [`SimulatedGpu`] is a cheap clonable handle onto shared state. A test can keep one clone to
//! inject faults while the service owns another as a `Box<dyn GpuDevice>`.

use crate::backend::{Backend, BackendError, DeviceError, DeviceResult, GpuDevice};
use crate::device::{
    Capabilities, ClockDomain, DeviceId, DeviceInfo, FanCaps, FanReading, OffsetCaps, PerfLimiter,
    PowerLimitCaps, Telemetry, TuningState, Vendor,
};
use crate::units::{Celsius, Mhz, MhzOffset, Milliwatts, Percent, Rpm};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SimConfig {
    pub name: String,
    pub ambient: f32,
    pub default_power_w: f32,
    pub min_power_w: f32,
    pub max_power_w: f32,
    /// Board power at 100 % load, before the power limit.
    pub full_load_power_w: f32,
    /// °C per watt with fans at 100 % / at 0 %.
    pub thermal_resistance_full_fan: f32,
    pub thermal_resistance_no_fan: f32,
    /// Thermal time constant in seconds.
    pub tau_secs: f32,
    pub fan_count: u32,
    pub fan_min_duty: f32,
    pub fan_max_rpm: u32,
    pub core_offset_range: (i32, i32),
    pub memory_offset_range: (i32, i32),
    pub slowdown_temp: f32,
    pub shutdown_temp: f32,
    pub base_core_mhz: u32,
    pub base_memory_mhz: u32,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            name: "Simulated GPU".into(),
            ambient: 25.0,
            default_power_w: 320.0,
            min_power_w: 100.0,
            max_power_w: 350.0,
            full_load_power_w: 340.0,
            thermal_resistance_full_fan: 0.12,
            thermal_resistance_no_fan: 0.45,
            tau_secs: 8.0,
            fan_count: 2,
            fan_min_duty: 30.0,
            fan_max_rpm: 3000,
            core_offset_range: (-1000, 1000),
            memory_offset_range: (-2000, 2000),
            slowdown_temp: 88.0,
            shutdown_temp: 95.0,
            base_core_mhz: 1860,
            base_memory_mhz: 9500,
        }
    }
}

/// Faults a test can switch on.
#[derive(Debug, Clone, Default)]
pub struct Faults {
    /// Writes succeed but the value doesn't change (the "accepted but not enforced" case).
    pub ignore_power_writes: bool,
    pub ignore_offset_writes: bool,
    pub ignore_fan_writes: bool,
    /// Fan duty writes return an error (firmware-auto still works).
    pub fan_writes_fail: bool,
    /// Every write returns `NoPermission`.
    pub permission_denied: bool,
    /// Temperature reads return nothing.
    pub temp_sensor_fails: bool,
    /// Every call returns `Lost`.
    pub device_lost: bool,
}

#[derive(Debug)]
struct SimState {
    temp: f32,
    load: f32,
    power_limit_mw: u32,
    core_offset: i32,
    memory_offset: i32,
    /// `None` = firmware auto.
    fan_target: Option<f32>,
    faults: Faults,
    /// `Some` = advance physics on wall-clock time at every telemetry read.
    wall_clock: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct SimulatedGpu {
    info: DeviceInfo,
    cfg: Arc<SimConfig>,
    state: Arc<Mutex<SimState>>,
}

impl SimulatedGpu {
    pub fn new(cfg: SimConfig) -> Self {
        SimulatedGpu::with_id(cfg, "sim:0")
    }

    pub fn with_id(cfg: SimConfig, id: &str) -> Self {
        let info = DeviceInfo {
            id: DeviceId(id.to_string()),
            vendor: Vendor::Simulated,
            name: cfg.name.clone(),
            architecture: Some("Simulated".into()),
            driver_version: Some("0.0".into()),
            vbios_version: None,
            pci_bus_id: None,
        };
        let state = SimState {
            temp: cfg.ambient,
            load: 0.0,
            power_limit_mw: Milliwatts::from_watts(cfg.default_power_w).0,
            core_offset: 0,
            memory_offset: 0,
            fan_target: None,
            faults: Faults::default(),
            wall_clock: None,
        };
        SimulatedGpu { info, cfg: Arc::new(cfg), state: Arc::new(Mutex::new(state)) }
    }

    /// Advance physics on wall-clock time whenever telemetry is read. For `--simulate` mode.
    pub fn real_time(self) -> Self {
        self.lock().wall_clock = Some(Instant::now());
        self
    }

    fn lock(&self) -> MutexGuard<'_, SimState> {
        // A poisoned lock means a test panicked mid-update; the state is still usable.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_load(&self, load: f32) {
        self.lock().load = load.clamp(0.0, 1.0);
    }

    pub fn set_temp(&self, temp: f32) {
        self.lock().temp = temp;
    }

    pub fn temp(&self) -> f32 {
        self.lock().temp
    }

    pub fn with_faults(&self, f: impl FnOnce(&mut Faults)) {
        f(&mut self.lock().faults);
    }

    /// Whether the firmware (not software) currently owns the fans.
    pub fn fans_on_firmware(&self) -> bool {
        self.lock().fan_target.is_none()
    }

    /// Advance the simulation by `dt`.
    pub fn step(&self, dt: Duration) {
        let mut s = self.lock();
        step_state(&self.cfg, &mut s, dt);
    }

    fn auto_duty(cfg: &SimConfig, temp: f32) -> f32 {
        (cfg.fan_min_duty + (temp - 45.0) * 2.5).clamp(cfg.fan_min_duty, 100.0)
    }

    fn fan_duty(cfg: &SimConfig, s: &SimState) -> f32 {
        s.fan_target.unwrap_or_else(|| SimulatedGpu::auto_duty(cfg, s.temp))
    }

    fn power_draw_w(cfg: &SimConfig, s: &SimState) -> f32 {
        let demand = s.load * cfg.full_load_power_w * (1.0 + s.core_offset as f32 / 4000.0) + 15.0;
        demand.min(s.power_limit_mw as f32 / 1000.0)
    }

    fn check(&self, s: &SimState) -> DeviceResult<()> {
        if s.faults.device_lost { Err(DeviceError::Lost) } else { Ok(()) }
    }

    fn check_write(&self, s: &SimState) -> DeviceResult<()> {
        self.check(s)?;
        if s.faults.permission_denied { Err(DeviceError::NoPermission) } else { Ok(()) }
    }
}

fn step_state(cfg: &SimConfig, s: &mut SimState, dt: Duration) {
    let fan = SimulatedGpu::fan_duty(cfg, s) / 100.0;
    let r = cfg.thermal_resistance_no_fan
        - (cfg.thermal_resistance_no_fan - cfg.thermal_resistance_full_fan) * fan;
    let mut power = SimulatedGpu::power_draw_w(cfg, s);
    if s.temp >= cfg.slowdown_temp {
        power *= 0.7; // firmware thermal slowdown
    }
    let steady = cfg.ambient + power * r;
    let alpha = 1.0 - (-dt.as_secs_f32() / cfg.tau_secs).exp();
    s.temp += (steady - s.temp) * alpha;
}

impl GpuDevice for SimulatedGpu {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn capabilities(&self) -> DeviceResult<Capabilities> {
        let s = self.lock();
        self.check(&s)?;
        let c = &self.cfg;
        Ok(Capabilities {
            power_limit: Some(PowerLimitCaps {
                min: Milliwatts::from_watts(c.min_power_w),
                max: Milliwatts::from_watts(c.max_power_w),
                default: Milliwatts::from_watts(c.default_power_w),
            }),
            core_offset: Some(OffsetCaps {
                min: MhzOffset(c.core_offset_range.0),
                max: MhzOffset(c.core_offset_range.1),
            }),
            memory_offset: Some(OffsetCaps {
                min: MhzOffset(c.memory_offset_range.0),
                max: MhzOffset(c.memory_offset_range.1),
            }),
            fans: (c.fan_count > 0).then(|| FanCaps {
                count: c.fan_count,
                min_duty: Percent::clamped(c.fan_min_duty),
                max_duty: Percent::FULL,
            }),
            slowdown_temp: Some(Celsius(c.slowdown_temp)),
            shutdown_temp: Some(Celsius(c.shutdown_temp)),
        })
    }

    fn telemetry(&self) -> DeviceResult<Telemetry> {
        let mut s = self.lock();
        self.check(&s)?;
        let c = &self.cfg;
        if let Some(last) = s.wall_clock {
            let now = Instant::now();
            step_state(c, &mut s, now - last);
            s.wall_clock = Some(now);
        }
        let power = SimulatedGpu::power_draw_w(c, &s);
        let duty = SimulatedGpu::fan_duty(c, &s);
        let mut limiters = Vec::new();
        if s.load < 0.05 {
            limiters.push(PerfLimiter::Idle);
        }
        if s.load >= 0.05 && power * 1000.0 >= s.power_limit_mw as f32 * 0.99 {
            limiters.push(PerfLimiter::Power);
        }
        if s.temp >= c.slowdown_temp {
            limiters.push(PerfLimiter::Thermal);
        }
        let core = if s.load < 0.05 {
            210
        } else {
            let mut mhz = c.base_core_mhz as i32 + s.core_offset;
            if limiters.contains(&PerfLimiter::Power) {
                mhz -= 60;
            }
            if limiters.contains(&PerfLimiter::Thermal) {
                mhz -= 400;
            }
            mhz.max(210)
        };
        Ok(Telemetry {
            gpu_temp: (!s.faults.temp_sensor_fails).then_some(Celsius(s.temp)),
            core_clock: Some(Mhz(core as u32)),
            memory_clock: Some(Mhz((c.base_memory_mhz as i32 + s.memory_offset).max(405) as u32)),
            power_draw: Some(Milliwatts::from_watts(power)),
            power_limit: Some(Milliwatts(s.power_limit_mw)),
            gpu_utilization: Some(Percent::clamped(s.load * 100.0)),
            memory_utilization: Some(Percent::clamped(s.load * 60.0)),
            vram_used_bytes: Some((1.5e9 + s.load as f64 * 6e9) as u64),
            vram_total_bytes: Some(12_000_000_000),
            fans: (0..c.fan_count)
                .map(|i| FanReading {
                    index: i,
                    duty: Some(Percent::clamped(duty)),
                    rpm: Some(Rpm((duty / 100.0 * c.fan_max_rpm as f32) as u32)),
                })
                .collect(),
            limiters,
            pstate: Some(if s.load < 0.05 { 8 } else { 0 }),
        })
    }

    fn tuning_state(&self) -> DeviceResult<TuningState> {
        let s = self.lock();
        self.check(&s)?;
        Ok(TuningState {
            power_limit: Some(Milliwatts(s.power_limit_mw)),
            core_offset: Some(MhzOffset(s.core_offset)),
            memory_offset: Some(MhzOffset(s.memory_offset)),
        })
    }

    fn set_power_limit(&mut self, limit: Milliwatts) -> DeviceResult<()> {
        let mut s = self.lock();
        self.check_write(&s)?;
        let (min, max) =
            (Milliwatts::from_watts(self.cfg.min_power_w), Milliwatts::from_watts(self.cfg.max_power_w));
        if limit < min || limit > max {
            return Err(DeviceError::InvalidValue(format!("{limit} outside {min}..={max}")));
        }
        if !s.faults.ignore_power_writes {
            s.power_limit_mw = limit.0;
        }
        Ok(())
    }

    fn set_clock_offset(&mut self, domain: ClockDomain, offset: MhzOffset) -> DeviceResult<()> {
        let mut s = self.lock();
        self.check_write(&s)?;
        let (lo, hi) = match domain {
            ClockDomain::Core => self.cfg.core_offset_range,
            ClockDomain::Memory => self.cfg.memory_offset_range,
        };
        if offset.0 < lo || offset.0 > hi {
            return Err(DeviceError::InvalidValue(format!("{offset} outside {lo}..={hi}")));
        }
        if !s.faults.ignore_offset_writes {
            match domain {
                ClockDomain::Core => s.core_offset = offset.0,
                ClockDomain::Memory => s.memory_offset = offset.0,
            }
        }
        Ok(())
    }

    fn set_fan_duty(&mut self, duty: Percent) -> DeviceResult<()> {
        let mut s = self.lock();
        self.check_write(&s)?;
        if self.cfg.fan_count == 0 {
            return Err(DeviceError::NotSupported);
        }
        if s.faults.fan_writes_fail {
            return Err(DeviceError::Other("simulated fan write failure".into()));
        }
        if duty.get() < self.cfg.fan_min_duty {
            return Err(DeviceError::InvalidValue(format!(
                "{duty} below the {} % minimum",
                self.cfg.fan_min_duty
            )));
        }
        if !s.faults.ignore_fan_writes {
            s.fan_target = Some(duty.get());
        }
        Ok(())
    }

    fn set_fans_auto(&mut self) -> DeviceResult<()> {
        let mut s = self.lock();
        self.check_write(&s)?;
        s.fan_target = None;
        Ok(())
    }
}

/// A backend that serves pre-built simulated devices.
#[derive(Debug, Default)]
pub struct SimBackend {
    devices: Vec<SimulatedGpu>,
}

impl SimBackend {
    pub fn new(devices: Vec<SimulatedGpu>) -> Self {
        SimBackend { devices }
    }
}

impl Backend for SimBackend {
    fn name(&self) -> &'static str {
        "simulated"
    }

    fn devices(&mut self) -> Result<Vec<Box<dyn GpuDevice>>, BackendError> {
        Ok(self.devices.iter().cloned().map(|d| Box::new(d) as Box<dyn GpuDevice>).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heats_under_load_and_cools_with_fans() {
        let gpu = SimulatedGpu::new(SimConfig::default());
        gpu.set_load(1.0);
        let mut quiet = gpu.clone();
        quiet.set_fan_duty(Percent::new(30.0).unwrap()).unwrap();
        for _ in 0..60 {
            gpu.step(Duration::from_secs(1));
        }
        let hot = gpu.temp();
        assert!(hot > 60.0, "expected heating, got {hot}");

        let mut loud = gpu.clone();
        loud.set_fan_duty(Percent::FULL).unwrap();
        for _ in 0..60 {
            gpu.step(Duration::from_secs(1));
        }
        assert!(gpu.temp() < hot, "100 % fans should cool: {} vs {hot}", gpu.temp());
    }

    #[test]
    fn power_limit_caps_draw() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        gpu.set_load(1.0);
        gpu.set_power_limit(Milliwatts::from_watts(200.0)).unwrap();
        let t = gpu.telemetry().unwrap();
        assert_eq!(t.power_draw, Some(Milliwatts::from_watts(200.0)));
        assert!(t.limiters.contains(&PerfLimiter::Power));
    }

    #[test]
    fn rejects_out_of_range_writes_like_a_driver() {
        let mut gpu = SimulatedGpu::new(SimConfig::default());
        assert!(matches!(gpu.set_fan_duty(Percent::new(10.0).unwrap()), Err(DeviceError::InvalidValue(_))));
        assert!(matches!(
            gpu.set_power_limit(Milliwatts::from_watts(999.0)),
            Err(DeviceError::InvalidValue(_))
        ));
    }
}
