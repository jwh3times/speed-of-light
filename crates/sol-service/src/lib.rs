//! The service owns the hardware. It's the only process that writes to GPUs. It runs fan
//! curves, enforces the safety watchdogs and serves clients over IPC.
//!
//! [`Service`] is synchronous and transport-free, so the whole control loop is tested against
//! simulated GPUs (`tests/service.rs`). [`host`] adds the tokio runtime and the named pipe,
//! shared by the console (`sol-service run`) and the Windows service in [`winsvc`].

pub mod backends;
pub mod cli;
pub mod conflicts;
#[cfg(windows)]
pub mod host;
pub mod registration;
#[cfg(windows)]
pub mod trust;
#[cfg(windows)]
pub mod winsvc;

use backends::Opened;
use conflicts::Conflict;
use sol_core::backend::{DeviceError, GpuDevice};
use sol_core::compat::{CompatMatrix, Feature};
use sol_core::crashguard::{CrashGuard, Sentinel};
use sol_core::device::{Capabilities, DeviceId, Telemetry};
use sol_core::fan::{FanCommand, FanController};
use sol_core::profile::{FanSetting, Profile};
use sol_core::tuner::{self, ApplyReport, Outcome, SafetyPolicy};
use sol_core::units::{Celsius, Percent};
use sol_ipc::{BackendStatus, DeviceStatus, FanModeView, PROTOCOL_VERSION, Request, Response};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// First wait before opening the backends again when no GPU was ready.
pub const RETRY_FIRST: Duration = Duration::from_secs(2);
/// The wait doubles after each miss, up to this.
pub const RETRY_MAX: Duration = Duration::from_secs(30);
/// How often running tools are checked for ones that fight over the same fans and clocks.
pub const CONFLICT_RECHECK: Duration = Duration::from_secs(30);

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
    pub startup_profile: Option<StartupProfile>,
    /// How long an applied profile must survive before the crash guard disarms.
    pub stability_window: Duration,
}

/// The startup profile as the host read it. The host decides whether the file can be trusted
/// (in service mode: only administrators can change it); the service only acts on the verdict.
#[derive(Debug, Clone)]
pub enum StartupProfile {
    Trusted(Profile),
    /// Not applied, for this reason (unreadable, invalid, or writable by non-admins).
    Rejected(String),
}

/// Where the service gets its GPUs. Polled until one is found (see [`RETRY_FIRST`]).
pub trait DeviceSource: Send {
    /// Try every backend once.
    fn open(&mut self) -> Opened;
}

impl<F: FnMut() -> Opened + Send> DeviceSource for F {
    fn open(&mut self) -> Opened {
        self()
    }
}

type ConflictDetector = Box<dyn FnMut() -> Vec<Conflict> + Send>;

#[derive(Debug, Clone, Copy)]
enum Acquire {
    /// No GPU yet, and some backend is worth trying again after `wait`.
    Waiting {
        wait: Duration,
        waited: Duration,
    },
    Found,
    /// No GPU, and no backend is worth trying again: status only.
    GaveUp,
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
    source: Box<dyn DeviceSource>,
    backends: Vec<BackendStatus>,
    acquire: Acquire,
    conflicts: Option<ConflictDetector>,
    since_conflict_check: Duration,
    /// Conflicting tools seen running at the last check.
    running_conflicts: Vec<Conflict>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("devices", &self.devices.len())
            .field("acquire", &self.acquire)
            .field("tripped", &self.tripped)
            .finish()
    }
}

impl Service {
    /// Opens `source` once now, and again from [`Service::tick`] with backoff until it finds a
    /// GPU. Each GPU found gets the startup takeover, then the startup profile is applied.
    pub fn new(source: impl DeviceSource + 'static, cfg: ServiceConfig, matrix: CompatMatrix) -> Self {
        let guard = CrashGuard::new(cfg.state_dir.join("crash-guard.json"));
        let tripped = match guard.check() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "can't read the crash guard; treating it as tripped");
                Some(Sentinel { profile: "<unreadable>".into(), armed_at_unix: 0, pid: 0 })
            }
        };
        let mut svc = Service {
            devices: Vec::new(),
            cfg,
            matrix,
            guard,
            tripped,
            armed_at: None,
            source: Box::new(source),
            backends: Vec::new(),
            acquire: Acquire::Waiting { wait: Duration::ZERO, waited: Duration::ZERO },
            conflicts: None,
            since_conflict_check: CONFLICT_RECHECK,
            running_conflicts: Vec::new(),
        };
        svc.poll_devices();
        svc
    }

    /// A service over devices that are already open (tests, and anything that can't wait).
    pub fn with_devices(devices: Vec<Box<dyn GpuDevice>>, cfg: ServiceConfig, matrix: CompatMatrix) -> Self {
        let mut devices = Some(devices);
        Service::new(
            move || Opened { devices: devices.take().unwrap_or_default(), statuses: Vec::new() },
            cfg,
            matrix,
        )
    }

    /// Check for other tuning tools from [`Service::tick`], every [`CONFLICT_RECHECK`].
    pub fn with_conflict_detector(mut self, detect: impl FnMut() -> Vec<Conflict> + Send + 'static) -> Self {
        self.conflicts = Some(Box::new(detect));
        self
    }

    fn poll_devices(&mut self) {
        let Acquire::Waiting { wait, .. } = self.acquire else { return };
        let opened = self.source.open();
        let retry = opened.worth_retrying();
        self.backends = opened.statuses;
        if !opened.devices.is_empty() {
            self.acquire = Acquire::Found;
            self.adopt(opened.devices);
            self.apply_startup_profile();
        } else if retry {
            let wait = if wait.is_zero() { RETRY_FIRST } else { (wait * 2).min(RETRY_MAX) };
            tracing::info!(retry_in = ?wait, "no GPU ready yet");
            self.acquire = Acquire::Waiting { wait, waited: Duration::ZERO };
        } else {
            self.acquire = Acquire::GaveUp;
            tracing::warn!("no controllable GPUs found; serving status only");
        }
    }

    fn apply_startup_profile(&mut self) {
        match self.startup() {
            Ok(reports) => {
                for r in reports {
                    tracing::info!(device = %r.device, profile = %r.profile, applied = r.all_applied(), "startup profile");
                }
            }
            Err(e) => {
                for m in &mut self.devices {
                    m.notice(format!("startup profile not applied: {e}"));
                }
            }
        }
    }

    fn adopt(&mut self, devices: Vec<Box<dyn GpuDevice>>) {
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
            // Tools seen while waiting for the driver (they start at login) concern this GPU too.
            for c in &self.running_conflicts {
                m.notice(conflict_notice(c));
            }
            self.devices.push(m);
        }
    }

    pub fn tripped(&self) -> Option<&Sentinel> {
        self.tripped.as_ref()
    }

    /// Apply the configured startup profile, unless it was rejected or the crash guard
    /// tripped. Runs by itself once GPUs are found; call it again after `sol guard clear`.
    pub fn startup(&mut self) -> Result<Vec<ApplyReport>, String> {
        let profile = match &self.cfg.startup_profile {
            None => return Ok(Vec::new()),
            Some(StartupProfile::Rejected(reason)) => return Err(reason.clone()),
            Some(StartupProfile::Trusted(p)) => p.clone(),
        };
        if let Some(t) = &self.tripped {
            return Err(format!(
                "crash guard tripped: profile {:?} did not survive its stability window last time. \
                 Leaving GPUs at stock. Run `sol guard clear` once you've fixed the profile.",
                t.profile
            ));
        }
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
                backends: self.backends.clone(),
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

    /// One control-loop iteration: look for GPUs if none are open yet, read telemetry, run fan
    /// curves, enforce watchdogs, and check for other tuning tools.
    pub fn tick(&mut self, dt: Duration) {
        if let Acquire::Waiting { wait, waited } = self.acquire {
            let waited = waited + dt;
            self.acquire = Acquire::Waiting { wait, waited };
            if waited >= wait {
                self.poll_devices();
            }
        }
        self.tick_conflicts(dt);

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

    fn tick_conflicts(&mut self, dt: Duration) {
        let Some(detect) = self.conflicts.as_mut() else { return };
        self.since_conflict_check += dt;
        if self.since_conflict_check < CONFLICT_RECHECK {
            return;
        }
        self.since_conflict_check = Duration::ZERO;
        let found: Vec<Conflict> = detect().into_iter().filter(Conflict::blocks_writes).collect();

        let mut messages = Vec::new();
        for c in &found {
            if !self.running_conflicts.iter().any(|r| r.tool == c.tool) {
                messages.push(conflict_notice(c));
            }
        }
        for gone in self.running_conflicts.iter().filter(|r| !found.iter().any(|c| c.tool == r.tool)) {
            messages.push(format!("{} is no longer running", gone.tool));
        }
        self.running_conflicts = found;
        for msg in messages {
            if self.devices.is_empty() {
                tracing::warn!("{msg}");
            }
            for m in &mut self.devices {
                m.notice(msg.clone());
            }
        }
    }

    /// Graceful stop: hand software-controlled fans back to firmware and disarm the guard.
    /// Power and clock settings stay applied; only [`reset_to_stock`] (uninstall) removes them.
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

fn conflict_notice(c: &Conflict) -> String {
    format!("{} is running ({}); it will fight this service over the same fans and clocks", c.tool, c.process)
}

/// What [`reset_to_stock`] did.
#[derive(Debug, Default)]
pub struct StockReset {
    pub reports: Vec<ApplyReport>,
    /// One line per GPU that isn't verifiably back at stock, or why nothing could be reset.
    pub failures: Vec<String>,
}

impl StockReset {
    /// Every GPU was reset and read back at stock, and there was at least one.
    pub fn ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Return every device to stock (default power limit, zero offsets, firmware control), as
/// uninstall does. Like every reset it is never gated by the compatibility matrix.
pub fn reset_to_stock(devices: Vec<Box<dyn GpuDevice>>) -> StockReset {
    let mut out = StockReset::default();
    if devices.is_empty() {
        out.failures.push("no GPU could be opened to reset".into());
    }
    for mut dev in devices {
        let id = dev.info().id.clone();
        let caps = match dev.capabilities() {
            Ok(c) => c,
            Err(e) => {
                out.failures.push(format!("{id}: capabilities unavailable ({e}); not reset"));
                continue;
            }
        };
        let report = tuner::reset(dev.as_mut(), &caps);
        let failed: Vec<String> = report
            .items
            .iter()
            .filter(|i| !i.outcome.is_success())
            .map(|i| format!("{} ({:?})", i.item, i.outcome))
            .collect();
        if !failed.is_empty() {
            out.failures.push(format!("{id}: not back at stock: {}", failed.join("; ")));
        }
        out.reports.push(report);
    }
    out
}
