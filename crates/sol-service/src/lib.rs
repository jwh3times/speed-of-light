//! The service owns the hardware. It's the only process that writes to GPUs. It runs fan
//! curves, enforces the safety watchdogs and serves clients over IPC.
//!
//! [`Service`] is synchronous and transport-free, so the whole control loop is tested against
//! simulated GPUs (`tests/service.rs`). `main.rs` adds the tokio runtime, the named pipe and
//! signal handling.

pub mod backends;
pub mod conflicts;

use sol_core::backend::{DeviceError, GpuDevice};
use sol_core::compat::{CompatMatrix, Feature};
use sol_core::crashguard::{CrashGuard, Sentinel};
use sol_core::device::{Capabilities, DeviceId, Telemetry};
use sol_core::fan::{FanCommand, FanController};
use sol_core::profile::{FanSetting, Profile};
use sol_core::tuner::{self, ApplyReport, Outcome, SafetyPolicy};
use sol_core::units::{Celsius, Percent};
use sol_ipc::{DeviceStatus, FanModeView, PROTOCOL_VERSION, Request, Response};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fixed-duty fans hand back to firmware this many degrees below the firmware slowdown point.
pub const FIXED_FAN_WATCHDOG_MARGIN: f32 = 5.0;
/// Fallback when the driver doesn't report a slowdown temperature.
pub const DEFAULT_CRITICAL_TEMP: Celsius = Celsius(90.0);
/// Consecutive failed fan writes before a curve gives up and hands back to firmware.
pub const MAX_FAN_WRITE_FAILURES: u32 = 3;
const MAX_NOTICES: usize = 20;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub state_dir: PathBuf,
    pub policy: SafetyPolicy,
    pub startup_profile: Option<PathBuf>,
    /// How long an applied profile must survive before the crash guard disarms.
    pub stability_window: Duration,
}

#[derive(Debug)]
enum FanMode {
    Firmware,
    Fixed(Percent),
    Curve { ctl: Box<FanController>, last_written: Option<Percent>, failures: u32 },
}

struct Managed {
    dev: Box<dyn GpuDevice>,
    caps: Option<Capabilities>,
    telemetry: Option<Telemetry>,
    telemetry_error: Option<String>,
    fan: FanMode,
    profile: Option<String>,
    notices: Vec<String>,
}

impl Managed {
    fn notice(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::warn!(device = %self.dev.info().id, "{msg}");
        self.notices.push(msg);
        if self.notices.len() > MAX_NOTICES {
            self.notices.remove(0);
        }
    }

    fn critical_temp(&self) -> Celsius {
        self.caps
            .as_ref()
            .and_then(|c| c.slowdown_temp)
            .map(|t| Celsius(t.0 - FIXED_FAN_WATCHDOG_MARGIN))
            .unwrap_or(DEFAULT_CRITICAL_TEMP)
    }

    fn fans_to_firmware(&mut self, why: &str) {
        if let Err(e) = self.dev.set_fans_auto() {
            self.notice(format!("{why}; returning fans to firmware FAILED: {e}"));
        } else if !why.is_empty() {
            self.notice(format!("{why}; fans returned to firmware control"));
        }
        self.fan = FanMode::Firmware;
    }

    fn status(&self) -> DeviceStatus {
        DeviceStatus {
            info: self.dev.info().clone(),
            capabilities: self.caps.clone(),
            telemetry: self.telemetry.clone(),
            telemetry_error: self.telemetry_error.clone(),
            fan_mode: match &self.fan {
                FanMode::Firmware => FanModeView::Firmware,
                FanMode::Fixed(d) => FanModeView::Fixed { duty: *d },
                FanMode::Curve { last_written, .. } => FanModeView::Curve { last_duty: *last_written },
            },
            active_profile: self.profile.clone(),
            notices: self.notices.clone(),
        }
    }
}

pub struct Service {
    devices: Vec<Managed>,
    cfg: ServiceConfig,
    matrix: CompatMatrix,
    guard: CrashGuard,
    tripped: Option<Sentinel>,
    armed_at: Option<Instant>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("devices", &self.devices.len())
            .field("tripped", &self.tripped)
            .finish()
    }
}

impl Service {
    pub fn new(devices: Vec<Box<dyn GpuDevice>>, cfg: ServiceConfig, matrix: CompatMatrix) -> Self {
        let guard = CrashGuard::new(cfg.state_dir.join("crash-guard.json"));
        let tripped = match guard.check() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "can't read the crash guard; treating it as tripped");
                Some(Sentinel { profile: "<unreadable>".into(), armed_at_unix: 0, pid: 0 })
            }
        };
        let mut svc = Service { devices: Vec::new(), cfg, matrix, guard, tripped, armed_at: None };
        for dev in devices {
            let caps = match dev.capabilities() {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!(device = %dev.info().id, error = %e, "capabilities unavailable");
                    None
                }
            };
            let mut m = Managed {
                dev,
                caps,
                telemetry: None,
                telemetry_error: None,
                fan: FanMode::Firmware,
                profile: None,
                notices: Vec::new(),
            };
            // A previous instance may have died holding the fans at a fixed duty. Take
            // ownership back and hand the fans to firmware before doing anything else.
            if m.caps.as_ref().is_some_and(|c| c.fans.is_some())
                && let Err(e) = m.dev.set_fans_auto()
            {
                tracing::debug!(device = %m.dev.info().id, error = %e, "could not reset fans at startup");
            }
            svc.devices.push(m);
        }
        svc
    }

    pub fn tripped(&self) -> Option<&Sentinel> {
        self.tripped.as_ref()
    }

    /// Apply the configured startup profile, unless the crash guard tripped.
    pub fn startup(&mut self) -> Result<Vec<ApplyReport>, String> {
        let Some(path) = self.cfg.startup_profile.clone() else { return Ok(Vec::new()) };
        if let Some(t) = &self.tripped {
            return Err(format!(
                "crash guard tripped: profile {:?} did not survive its stability window last time. \
                 Leaving GPUs at stock. Run `sol guard clear` once you've fixed the profile.",
                t.profile
            ));
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let profile = Profile::from_toml(&text).map_err(|e| e.to_string())?;
        Ok(self.apply(&profile, false))
    }

    pub fn handle(&mut self, req: Request) -> Response {
        match req {
            Request::Hello { protocol } if protocol == PROTOCOL_VERSION => {
                Response::Hello { protocol: PROTOCOL_VERSION, service_version: VERSION.into() }
            }
            Request::Hello { protocol } => Response::Error {
                message: format!("protocol {protocol} not supported; service speaks {PROTOCOL_VERSION}"),
            },
            Request::Status => Response::Status {
                devices: self.devices.iter().map(Managed::status).collect(),
                guard: self.tripped.clone(),
            },
            Request::ApplyProfile { profile, allow_unverified } => match profile.validate() {
                Err(e) => Response::Error { message: e.to_string() },
                Ok(()) => Response::Applied { reports: self.apply(&profile, allow_unverified) },
            },
            Request::Reset { device } => Response::Applied { reports: self.reset(device.as_ref()) },
            Request::SetFan { device, setting, allow_unverified } => {
                let profile = Profile {
                    schema: sol_core::profile::SCHEMA_VERSION,
                    name: "manual fan".into(),
                    target: sol_core::profile::DeviceMatch { id: device, name_contains: None },
                    power: None,
                    clocks: Default::default(),
                    fan: Some(setting),
                };
                match profile.validate() {
                    Err(e) => Response::Error { message: e.to_string() },
                    Ok(()) => Response::Applied { reports: self.apply(&profile, allow_unverified) },
                }
            }
            Request::GuardStatus => Response::Guard { tripped: self.tripped.clone() },
            Request::GuardClear => match self.guard.disarm() {
                Ok(()) => {
                    self.tripped = None;
                    Response::Ok { message: "crash guard cleared".into() }
                }
                Err(e) => Response::Error { message: format!("can't clear the crash guard: {e}") },
            },
        }
    }

    fn apply(&mut self, profile: &Profile, allow_unverified: bool) -> Vec<ApplyReport> {
        let policy = SafetyPolicy {
            allow_unverified: self.cfg.policy.allow_unverified || allow_unverified,
            ..self.cfg.policy.clone()
        };
        let touches_hardware = profile.power.is_some()
            || profile.clocks.core_offset_mhz.is_some()
            || profile.clocks.memory_offset_mhz.is_some();
        if touches_hardware {
            if let Err(e) = self.guard.arm(&profile.name) {
                tracing::error!(error = %e, "can't arm the crash guard");
            }
            self.armed_at = Some(Instant::now());
        }

        let mut reports = Vec::new();
        for m in self.devices.iter_mut().filter(|m| profile.target.matches(m.dev.info())) {
            let Some(caps) = m.caps.clone() else {
                reports.push(ApplyReport {
                    device: m.dev.info().id.clone(),
                    profile: profile.name.clone(),
                    items: vec![tuner::ItemReport {
                        item: "profile".into(),
                        feature: Feature::Telemetry,
                        outcome: Outcome::Failed { error: "device capabilities unavailable".into() },
                    }],
                });
                continue;
            };
            let plan = tuner::plan(profile, &caps, &policy);
            let report = tuner::execute(m.dev.as_mut(), &profile.name, &plan, &self.matrix, &policy);

            let fan_ok = |item: &str| {
                report.items.iter().any(|i| {
                    i.feature == Feature::FanControl && i.item.starts_with(item) && i.outcome.is_success()
                })
            };
            match (&profile.fan, plan.fan_curve) {
                (Some(FanSetting::Curve { .. }), Some((curve, tuning))) if fan_ok("fan curve") => {
                    m.fan = FanMode::Curve {
                        ctl: Box::new(FanController::new(curve, tuning)),
                        last_written: None,
                        failures: 0,
                    };
                }
                (Some(FanSetting::Fixed { .. }), _) => {
                    if let Some(tuner::Action::SetFanDuty { duty }) =
                        plan.actions.iter().find(|a| matches!(a, tuner::Action::SetFanDuty { .. }))
                        && fan_ok("fans fixed")
                    {
                        m.fan = FanMode::Fixed(*duty);
                    }
                }
                (Some(FanSetting::Auto), _) if fan_ok("fans auto") => m.fan = FanMode::Firmware,
                _ => {}
            }
            m.profile = Some(profile.name.clone());
            reports.push(report);
        }
        reports
    }

    fn reset(&mut self, device: Option<&DeviceId>) -> Vec<ApplyReport> {
        let mut reports = Vec::new();
        for m in self.devices.iter_mut().filter(|m| device.is_none_or(|id| *id == m.dev.info().id)) {
            let Some(caps) = m.caps.clone() else { continue };
            reports.push(tuner::reset(m.dev.as_mut(), &caps));
            m.fan = FanMode::Firmware;
            m.profile = None;
        }
        reports
    }

    /// One control-loop iteration: read telemetry, run fan curves, enforce watchdogs.
    pub fn tick(&mut self, dt: Duration) {
        for m in &mut self.devices {
            match m.dev.telemetry() {
                Ok(t) => {
                    m.telemetry = Some(t);
                    m.telemetry_error = None;
                }
                Err(e) => {
                    m.telemetry_error = Some(e.to_string());
                    if e == DeviceError::Lost {
                        m.telemetry = None;
                    }
                }
            }
            let temp = if m.telemetry_error.is_none() {
                m.telemetry.as_ref().and_then(|t| t.gpu_temp)
            } else {
                None
            };
            let critical = m.critical_temp();

            match &mut m.fan {
                FanMode::Firmware => {}
                FanMode::Fixed(_) => {
                    // A fixed duty can't react to load. Above the watchdog point (or with no
                    // temperature at all), the firmware's own curve is the safer owner.
                    match temp {
                        Some(t) if t.0 >= critical.0 => {
                            m.fans_to_firmware(&format!("fixed fan watchdog: {t} >= {critical}"))
                        }
                        None => m.fans_to_firmware("fixed fan watchdog: no temperature reading"),
                        Some(_) => {}
                    }
                }
                FanMode::Curve { ctl, last_written, failures } => match ctl.update(temp, dt) {
                    FanCommand::FirmwareAuto => {
                        m.fans_to_firmware("fan curve: temperature sensor unavailable")
                    }
                    FanCommand::Duty(d) => {
                        let duty = m.caps.as_ref().and_then(|c| c.fans).map_or(d, |fc| fc.clamp(d));
                        let changed = last_written.is_none_or(|w| (w.get() - duty.get()).abs() >= 1.0);
                        if changed {
                            match m.dev.set_fan_duty(duty) {
                                Ok(()) => {
                                    *last_written = Some(duty);
                                    *failures = 0;
                                }
                                Err(e) => {
                                    *failures += 1;
                                    if *failures >= MAX_FAN_WRITE_FAILURES {
                                        m.fans_to_firmware(&format!(
                                            "fan curve: {MAX_FAN_WRITE_FAILURES} failed writes ({e})"
                                        ));
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }

        if let Some(at) = self.armed_at
            && at.elapsed() >= self.cfg.stability_window
        {
            match self.guard.disarm() {
                Ok(()) => {
                    tracing::info!("profile survived the stability window; crash guard disarmed");
                    self.armed_at = None;
                }
                Err(e) => tracing::error!(error = %e, "can't disarm the crash guard"),
            }
        }
    }

    /// Graceful stop: hand software-controlled fans back to firmware and disarm the guard.
    pub fn shutdown(&mut self) {
        for m in &mut self.devices {
            if !matches!(m.fan, FanMode::Firmware) {
                m.fans_to_firmware("service stopping");
            }
        }
        if let Err(e) = self.guard.disarm() {
            tracing::error!(error = %e, "can't disarm the crash guard on shutdown");
        }
    }
}
