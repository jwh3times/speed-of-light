//! End-to-end control-loop tests against simulated GPUs. No hardware, no pipes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sol_core::backend::GpuDevice;
use sol_core::compat::CompatMatrix;
use sol_core::profile::{FanSetting, Profile};
use sol_core::sim::{SimConfig, SimulatedGpu};
use sol_core::tuner::{Outcome, SafetyPolicy};
use sol_core::units::{MhzOffset, Percent};
use sol_ipc::{FanModeView, PROTOCOL_VERSION, Request, Response};
use sol_service::{Service, ServiceConfig};
use std::path::Path;
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
    Service::new(vec![Box::new(gpu.clone())], config(dir), CompatMatrix::default())
}

fn profile(body: &str) -> Profile {
    Profile::from_toml(&format!("schema = 1\nname = \"test\"\n{body}")).unwrap()
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
    let profile_path = dir.path().join("oc.toml");
    std::fs::write(&profile_path, "schema = 1\nname = \"oc\"\n[clocks]\ncore_offset_mhz = 150\n").unwrap();
    let gpu = SimulatedGpu::new(SimConfig::default());

    // First boot: apply, then "crash" (drop without shutdown, inside the stability window).
    {
        let mut cfg = config(dir.path());
        cfg.startup_profile = Some(profile_path.clone());
        let mut svc = Service::new(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());
        let reports = svc.startup().unwrap();
        assert!(reports[0].all_applied());
    }

    // The reboot resets the card; the driver doesn't keep offsets.
    let gpu = SimulatedGpu::new(SimConfig::default());
    let mut cfg = config(dir.path());
    cfg.startup_profile = Some(profile_path);
    let mut svc = Service::new(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());
    assert_eq!(svc.tripped().map(|s| s.profile.as_str()), Some("oc"));
    let err = svc.startup().unwrap_err();
    assert!(err.contains("crash guard"), "{err}");
    assert_eq!(gpu.tuning_state().unwrap().core_offset, Some(MhzOffset(0)));

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
    let mut svc = Service::new(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());
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
    let mut svc = Service::new(vec![Box::new(gpu.clone())], cfg, CompatMatrix::default());

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
    let mut svc = Service::new(
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
