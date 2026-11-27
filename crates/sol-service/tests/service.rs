//! End-to-end control-loop tests against simulated GPUs. No hardware, no pipes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sol_core::backend::GpuDevice;
use sol_core::compat::CompatMatrix;
use sol_core::profile::{FanSetting, Profile};
use sol_core::sim::{SimConfig, SimulatedGpu};
use sol_core::tuner::{Outcome, SafetyPolicy};
use sol_core::units::{MhzOffset, Percent};
use sol_ipc::{BackendState, BackendStatus, FanModeView, PROTOCOL_VERSION, Request, Response};
use sol_service::backends::Opened;
use sol_service::{DeviceSource, Service, ServiceConfig, StartupProfile};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const SEC: Duration = Duration::from_secs(1);

fn config(dir: &Path) -> ServiceConfig {
    ServiceConfig {
        state_dir: dir.to_path_buf(),
        policy: SafetyPolicy { allow_unverified: true, ..SafetyPolicy::default() },
        startup_profile: None,
        stability_window: Duration::from_secs(3600),
    }
}

fn service(dir: &Path, gpu: &SimulatedGpu) -> Service {
    Service::with_devices(vec![Box::new(gpu.clone())], config(dir), CompatMatrix::default())
}

fn profile(body: &str) -> Profile {
    profile_named("test", body)
}

fn profile_named(name: &str, body: &str) -> Profile {
    Profile::from_toml(&format!("schema = 1\nname = \"{name}\"\n{body}")).unwrap()
}

fn backend(status: BackendState) -> BackendStatus {
    BackendStatus { backend: "test".into(), status, detail: format!("{status:?}"), devices: 0 }
}

/// A device source that is `NotReady` for the first `not_ready` polls, then finds `gpu`.
/// Returns how many times it was polled.
fn slow_driver(not_ready: usize, gpu: &SimulatedGpu) -> (impl DeviceSource + 'static, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polls);
    let gpu = gpu.clone();
    let source = move || {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        if n < not_ready {
            Opened { devices: Vec::new(), statuses: vec![backend(BackendState::NotReady)] }
        } else {
            let mut ok = backend(BackendState::Ok);
            ok.devices = 1;
            Opened { devices: vec![Box::new(gpu.clone())], statuses: vec![ok] }
        }
    };
    (source, polls)
}

fn status(svc: &mut Service) -> (Vec<sol_ipc::DeviceStatus>, Vec<BackendStatus>) {
    match svc.handle(Request::Status) {
        Response::Status { devices, backends, .. } => (devices, backends),
        other => panic!("expected Status, got {other:?}"),
    }
}

fn polls(p: &Arc<AtomicUsize>) -> usize {
    p.load(Ordering::SeqCst)
}

fn apply(svc: &mut Service, p: Profile) -> Vec<sol_core::tuner::ApplyReport> {
    match svc.handle(Request::ApplyProfile { profile: p, allow_unverified: false }) {
        Response::Applied { reports } => reports,
        other => panic!("expected Applied, got {other:?}"),
    }
}

fn fan_mode(svc: &mut Service) -> FanModeView {
    match svc.handle(Request::Status) {
        Response::Status { devices, .. } => devices[0].fan_mode.clone(),
        other => panic!("expected Status, got {other:?}"),
    }
}

fn notices(svc: &mut Service) -> Vec<String> {
    match svc.handle(Request::Status) {
        Response::Status { devices, .. } => devices[0].notices.clone(),
        other => panic!("expected Status, got {other:?}"),
    }
}

/// Advance the simulation and the service together.
fn run(svc: &mut Service, gpu: &SimulatedGpu, secs: u32) -> f32 {
    let mut max_temp: f32 = 0.0;
    for _ in 0..secs {
        gpu.step(SEC);
        svc.tick(SEC);
        max_temp = max_temp.max(gpu.temp());
    }
    max_temp
}

const CURVE: &str = "[fan]\nmode = \"curve\"\npoints = [[40, 30], [60, 50], [75, 100]]\n";

#[test]
fn fan_curve_keeps_a_loaded_gpu_out_of_thermal_slowdown() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    gpu.set_load(1.0);
    let mut svc = service(dir.path(), &gpu);
    assert!(apply(&mut svc, profile(CURVE))[0].all_applied());

    let max = run(&mut svc, &gpu, 600);
    assert!(max < 80.0, "curve should hold the GPU well under slowdown, peaked at {max:.1}");
    assert!(matches!(fan_mode(&mut svc), FanModeView::Curve { last_duty: Some(_) }));
    assert!(!gpu.fans_on_firmware());
}

#[test]
fn a_too_quiet_fixed_fan_is_handed_back_to_firmware_before_slowdown() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    gpu.set_load(1.0);
    let mut svc = service(dir.path(), &gpu);
    assert!(apply(&mut svc, profile("[fan]\nmode = \"fixed\"\nduty = 30\n"))[0].all_applied());
    assert!(matches!(fan_mode(&mut svc), FanModeView::Fixed { .. }));

    let max = run(&mut svc, &gpu, 600);
    assert!(max < SimConfig::default().slowdown_temp, "watchdog let it reach slowdown: {max:.1}");
    assert_eq!(fan_mode(&mut svc), FanModeView::Firmware);
    assert!(gpu.fans_on_firmware());
    assert!(notices(&mut svc).iter().any(|n| n.contains("watchdog")));
}

#[test]
fn curve_hands_back_to_firmware_when_the_sensor_dies() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut svc = service(dir.path(), &gpu);
    apply(&mut svc, profile(CURVE));
    run(&mut svc, &gpu, 5);
    assert!(!gpu.fans_on_firmware());

    gpu.with_faults(|f| f.temp_sensor_fails = true);
    run(&mut svc, &gpu, 3);
    assert!(gpu.fans_on_firmware());
    assert_eq!(fan_mode(&mut svc), FanModeView::Firmware);
}

#[test]
fn curve_hands_back_to_firmware_after_repeated_write_failures() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    gpu.set_load(1.0);
    let mut svc = service(dir.path(), &gpu);
    apply(&mut svc, profile(CURVE));
    run(&mut svc, &gpu, 2);
    gpu.with_faults(|f| f.fan_writes_fail = true);
    // The duty has to change for the loop to write; heating guarantees that.
    run(&mut svc, &gpu, 60);
    assert_eq!(fan_mode(&mut svc), FanModeView::Firmware);
    assert!(gpu.fans_on_firmware());
}

#[test]
fn startup_takes_fans_back_from_a_dead_previous_instance() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    gpu.clone().set_fan_duty(Percent::new(40.0).unwrap()).unwrap();
    assert!(!gpu.fans_on_firmware());
    let _svc = service(dir.path(), &gpu);
    assert!(gpu.fans_on_firmware());
}

#[test]
fn graceful_shutdown_returns_fans_and_disarms_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut svc = service(dir.path(), &gpu);
    apply(&mut svc, profile(&format!("[power]\npercent = 90\n{CURVE}")));
    run(&mut svc, &gpu, 3);
    svc.shutdown();
    assert!(gpu.fans_on_firmware());
    assert!(!dir.path().join("crash-guard.json").exists());
}

#[test]
fn crash_guard_blocks_the_startup_profile_after_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let oc = profile_named("oc", "[clocks]\ncore_offset_mhz = 150\n");
    let gpu = SimulatedGpu::new(SimConfig::default());

    // First boot: the startup profile applies by itself, then the PC "crashes" (the service
    // is dropped without a graceful stop, inside the stability window).
    {
        let mut cfg = config(dir.path());
        cfg.startup_profile = Some(StartupProfile::Trusted(oc.clone()));
        let _svc = Service::with_devices(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());
        assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(150)));
    }

    // The reboot resets the card; the driver doesn't keep offsets.
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut cfg = config(dir.path());
    cfg.startup_profile = Some(StartupProfile::Trusted(oc));
    let mut svc = Service::with_devices(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());
    assert_eq!(svc.tripped().map(|s| s.profile.as_str()), Some("oc"));
    assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(0)));
    assert!(notices(&mut svc).iter().any(|n| n.contains("crash guard")), "{:?}", notices(&mut svc));
    let err = svc.startup().unwrap_err();
    assert!(err.contains("crash guard"), "{err}");

    // The user fixes things and clears the guard.
    assert!(matches!(svc.handle(Request::GuardClear), Response::Ok { .. }));
    assert!(svc.startup().unwrap()[0].all_applied());
}

#[test]
fn guard_disarms_once_the_stability_window_passes() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut cfg = config(dir.path());
    cfg.stability_window = Duration::ZERO;
    let mut svc = Service::with_devices(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());
    apply(&mut svc, profile("[power]\npercent = 95\n"));
    assert!(dir.path().join("crash-guard.json").exists());
    svc.tick(SEC);
    assert!(!dir.path().join("crash-guard.json").exists());
}

#[test]
fn unverified_writes_are_blocked_unless_the_request_opts_in() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut cfg = config(dir.path());
    cfg.policy.allow_unverified = false;
    let mut svc = Service::with_devices(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());

    let p = profile("[clocks]\ncore_offset_mhz = 60\n");
    let reports = apply(&mut svc, p.clone());
    assert!(matches!(reports[0].items[0].outcome, Outcome::Blocked { .. }));

    let Response::Applied { reports } =
        svc.handle(Request::ApplyProfile { profile: p, allow_unverified: true })
    else {
        panic!()
    };
    assert!(reports[0].all_applied());
    assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(60)));
}

#[test]
fn reset_returns_to_stock() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut svc = service(dir.path(), &gpu);
    apply(&mut svc, profile(&format!("[power]\npercent = 80\n[clocks]\nmemory_offset_mhz = 400\n{CURVE}")));
    let Response::Applied { reports } = svc.handle(Request::Reset { device: None }) else { panic!() };
    assert!(reports[0].all_applied(), "{reports:#?}");
    assert_eq!(gpu.tuning_state().unwrap().memory_offset, Some(MhzOffset(0)));
    assert_eq!(fan_mode(&mut svc), FanModeView::Firmware);
}

#[test]
fn set_fan_request_and_profile_targeting() {
    let dir = tempfile::tempdir().unwrap();
    let a = SimulatedGpu::with_id(SimConfig { name: "Alpha".into(), ..SimConfig::default() }, "sim:a");
    let b = SimulatedGpu::with_id(SimConfig { name: "Beta".into(), ..SimConfig::default() }, "sim:b");
    let mut svc = Service::with_devices(
        vec![Box::new(a.clone()), Box::new(b.clone())],
        config(dir.path()),
        CompatMatrix::default(),
    );

    let r = svc.handle(Request::SetFan {
        device: Some(sol_core::DeviceId("sim:b".into())),
        setting: FanSetting::Fixed { duty: Percent::new(70.0).unwrap() },
        allow_unverified: false,
    });
    let Response::Applied { reports } = r else { panic!("{r:?}") };
    assert_eq!(reports.len(), 1);
    assert!(a.fans_on_firmware());
    assert!(!b.fans_on_firmware());
}

#[test]
fn hello_checks_the_protocol_version() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut svc = service(dir.path(), &gpu);
    assert!(matches!(svc.handle(Request::Hello { protocol: PROTOCOL_VERSION }), Response::Hello { .. }));
    assert!(matches!(svc.handle(Request::Hello { protocol: 999 }), Response::Error { .. }));
}

#[test]
fn the_startup_profile_waits_for_a_gpu_that_is_not_ready_yet() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    // A previous instance died holding the fans.
    gpu.clone().set_fan_duty(Percent::new(40.0).unwrap()).unwrap();
    let (source, polled) = slow_driver(2, &gpu);
    let mut cfg = config(dir.path());
    cfg.startup_profile = Some(StartupProfile::Trusted(profile("[clocks]\ncore_offset_mhz = 100\n")));
    let mut svc = Service::new(source, cfg, CompatMatrix::default());

    // Polled once at start: nothing yet, and clients can see why.
    assert_eq!(polls(&polled), 1);
    let (devices, backends) = status(&mut svc);
    assert!(devices.is_empty());
    assert_eq!(backends[0].status, BackendState::NotReady);

    // The first retry comes 2 s later, the next one 4 s after that.
    svc.tick(SEC);
    assert_eq!(polls(&polled), 1);
    svc.tick(SEC);
    assert_eq!(polls(&polled), 2);
    for _ in 0..3 {
        svc.tick(SEC);
    }
    assert_eq!(polls(&polled), 2);
    assert!(!gpu.fans_on_firmware(), "nothing touches a GPU the service hasn't found");
    svc.tick(SEC);
    assert_eq!(polls(&polled), 3);

    // Found: the startup takeover runs, then the startup profile applies.
    assert!(gpu.fans_on_firmware());
    assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(100)));
    let (devices, backends) = status(&mut svc);
    assert_eq!(devices.len(), 1);
    assert_eq!(backends[0].status, BackendState::Ok);

    // Once a GPU is found the source isn't polled again.
    for _ in 0..120 {
        svc.tick(SEC);
    }
    assert_eq!(polls(&polled), 3);
}

#[test]
fn the_retry_wait_doubles_up_to_thirty_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let (source, polled) = slow_driver(usize::MAX, &gpu);
    let mut svc = Service::new(source, config(dir.path()), CompatMatrix::default());
    // Waits of 2, 4, 8, 16, 30, 30 s: polls land at 2, 6, 14, 30, 60 and 90 s after start.
    let mut seen = Vec::new();
    for second in 1..=90 {
        svc.tick(SEC);
        if polls(&polled) > seen.len() + 1 {
            seen.push(second);
        }
    }
    assert_eq!(seen, vec![2, 6, 14, 30, 60, 90]);
}

#[test]
fn a_rejected_startup_profile_is_not_applied_and_a_notice_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut cfg = config(dir.path());
    cfg.startup_profile =
        Some(StartupProfile::Rejected("non-administrators can change it (BUILTIN\\Users)".into()));
    let mut svc = Service::with_devices(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());
    assert!(
        notices(&mut svc)
            .iter()
            .any(|n| n.starts_with("startup profile not applied") && n.contains("non-administrators")),
        "{:?}",
        notices(&mut svc)
    );
    assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(0)));
    assert!(!dir.path().join("crash-guard.json").exists());
}

#[test]
fn a_tuning_tool_started_after_the_service_gets_a_notice_and_fan_writes_continue() {
    use sol_service::conflicts::{Conflict, Role};
    use std::sync::Mutex;

    let dir = tempfile::tempdir().unwrap();
    let a = SimulatedGpu::with_id(SimConfig::default(), "sim:a");
    let b = SimulatedGpu::with_id(SimConfig::default(), "sim:b");
    a.set_load(1.0);
    let running: Arc<Mutex<Vec<Conflict>>> = Arc::default();
    let seen = Arc::clone(&running);
    let mut svc = Service::with_devices(
        vec![Box::new(a.clone()), Box::new(b.clone())],
        config(dir.path()),
        CompatMatrix::default(),
    )
    .with_conflict_detector(move || seen.lock().unwrap().clone());
    apply(&mut svc, profile(CURVE));
    run(&mut svc, &a, 5);

    // Afterburner starts at login, after the service.
    running.lock().unwrap().push(Conflict {
        tool: "MSI Afterburner",
        process: "MSIAfterburner.exe".into(),
        role: Role::Tuning,
    });
    // An overlay doesn't conflict and gets no notice.
    running.lock().unwrap().push(Conflict {
        tool: "RivaTuner Statistics Server",
        process: "RTSS.exe".into(),
        role: Role::Overlay,
    });
    // Still running through two more checks: it is reported once, not at every check.
    run(&mut svc, &a, 30 + 2 * 30);

    let (devices, _) = status(&mut svc);
    assert_eq!(devices.len(), 2);
    for d in &devices {
        let about_afterburner: Vec<_> = d.notices.iter().filter(|n| n.contains("MSI Afterburner")).collect();
        assert_eq!(about_afterburner.len(), 1, "{:?}", d.notices);
        assert!(!d.notices.iter().any(|n| n.contains("RivaTuner")), "{:?}", d.notices);
    }
    assert!(!a.fans_on_firmware(), "the service keeps driving its fans");
    assert!(matches!(fan_mode(&mut svc), FanModeView::Curve { last_duty: Some(_) }));

    // It goes away again.
    running.lock().unwrap().clear();
    run(&mut svc, &a, 30);
    assert!(
        notices(&mut svc).iter().any(|n| n == "MSI Afterburner is no longer running"),
        "{:?}",
        notices(&mut svc)
    );
}

#[test]
fn uninstall_returns_every_gpu_to_stock() {
    let a = SimulatedGpu::with_id(SimConfig::default(), "sim:a");
    let b = SimulatedGpu::with_id(SimConfig::default(), "sim:b");
    let stock_limit = a.tuning_state().unwrap().power_limit;
    for gpu in [&a, &b] {
        let mut g = gpu.clone();
        g.set_power_limit(sol_core::units::Milliwatts(250_000)).unwrap();
        g.set_clock_offset(sol_core::device::ClockDomain::Memory, MhzOffset(500)).unwrap();
        g.set_fan_duty(Percent::new(35.0).unwrap()).unwrap();
    }

    let result = sol_service::reset_to_stock(vec![Box::new(a.clone()), Box::new(b.clone())]);
    assert!(result.ok(), "{result:?}");
    assert_eq!(result.reports.len(), 2);
    for gpu in [&a, &b] {
        let t = gpu.tuning_state().unwrap();
        assert_eq!(t.memory_offset, Some(MhzOffset(0)));
        assert_eq!(t.power_limit, stock_limit);
        assert!(gpu.fans_on_firmware());
    }
}

#[test]
fn uninstall_reports_a_gpu_it_could_not_reset() {
    let good = SimulatedGpu::with_id(SimConfig::default(), "sim:good");
    let bad = SimulatedGpu::with_id(SimConfig::default(), "sim:bad");
    bad.with_faults(|f| f.permission_denied = true);
    let result = sol_service::reset_to_stock(vec![Box::new(good), Box::new(bad)]);
    assert!(!result.ok());
    assert_eq!(result.failures.len(), 1, "{result:?}");
    assert!(result.failures[0].contains("sim:bad"), "{result:?}");

    let nothing = sol_service::reset_to_stock(Vec::new());
    assert!(!nothing.ok(), "with no GPU to reset, uninstall must not claim it returned to stock");
}

#[test]
fn a_tuning_tool_seen_before_the_gpu_is_ready_is_reported_on_the_gpu_found_later() {
    use sol_service::conflicts::{Conflict, Role};

    let dir = tempfile::tempdir().unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());
    let (source, _) = slow_driver(5, &gpu);
    let mut svc =
        Service::new(source, config(dir.path()), CompatMatrix::default()).with_conflict_detector(|| {
            vec![Conflict { tool: "Fan Control", process: "FanControl.exe".into(), role: Role::FanControl }]
        });
    // At boot: the tool is already running, the driver isn't ready yet.
    svc.tick(SEC);
    assert!(status(&mut svc).0.is_empty());
    for _ in 0..60 {
        svc.tick(SEC);
    }
    let (devices, _) = status(&mut svc);
    assert_eq!(devices.len(), 1);
    assert!(
        devices[0].notices.iter().any(|n| n.contains("Fan Control is running")),
        "{:?}",
        devices[0].notices
    );
}

#[test]
fn an_unexpected_backend_error_is_retried_too() {
    let dir = tempfile::tempdir().unwrap();
    let polled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polled);
    let source = move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Opened { devices: Vec::new(), statuses: vec![backend(BackendState::Error)] }
    };
    let mut svc = Service::new(source, config(dir.path()), CompatMatrix::default());
    for _ in 0..10 {
        svc.tick(SEC);
    }
    assert!(polls(&polled) > 1, "only a missing or unusable runtime is given up on");
}

#[test]
fn a_missing_runtime_is_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let polled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polled);
    let source = move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Opened { devices: Vec::new(), statuses: vec![backend(BackendState::NotPresent)] }
    };
    let mut svc = Service::new(source, config(dir.path()), CompatMatrix::default());
    for _ in 0..120 {
        svc.tick(SEC);
    }
    assert_eq!(polls(&polled), 1);
    let (devices, backends) = status(&mut svc);
    assert!(devices.is_empty());
    assert_eq!(backends[0].status, BackendState::NotPresent);
}
