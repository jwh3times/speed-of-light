//! Commands that open the hardware directly instead of going through the service.
//! `probe` and `doctor` only read. `selftest --write` writes, verifies and restores.

use crate::format;
use anyhow::{Result, bail};
use serde::Serialize;
use sol_core::backend::GpuDevice;
use sol_core::compat::{CompatMatrix, Feature, Verdict};
use sol_core::device::{Capabilities, ClockDomain, DeviceInfo, Telemetry, TuningState, Vendor};
use sol_core::tuner::{self, Action, Outcome};
use sol_core::units::{MhzOffset, Milliwatts, Percent};
use sol_service::backends::{self, BackendStatus};
use sol_service::conflicts::{self, Conflict};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn probe(simulate: Option<u32>, json: bool) -> Result<()> {
    let opened = backends::open_all(simulate, 0.0);
    if json {
        #[derive(Serialize)]
        struct Dev {
            info: DeviceInfo,
            capabilities: Option<Capabilities>,
            telemetry: Option<Telemetry>,
        }
        #[derive(Serialize)]
        struct Out<'a> {
            backends: &'a [BackendStatus],
            devices: Vec<Dev>,
        }
        let devices = opened
            .devices
            .iter()
            .map(|d| Dev {
                info: d.info().clone(),
                capabilities: d.capabilities().ok(),
                telemetry: d.telemetry().ok(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&Out { backends: &opened.statuses, devices })?);
        return Ok(());
    }
    for s in &opened.statuses {
        println!("{:<12} {:<12} {}", s.backend, s.status, s.detail);
    }
    for d in &opened.devices {
        println!();
        print!("{}", format::device_block(d.info(), d.capabilities().ok().as_ref()));
        match d.telemetry() {
            Ok(t) => println!("  {}", format::telemetry_line("now", &t)),
            Err(e) => println!("  telemetry error: {e}"),
        }
    }
    let found = conflicts::detect();
    if !found.is_empty() {
        println!();
        for c in found {
            let note = if c.blocks_writes() {
                "will conflict with tuning/fan writes"
            } else {
                "overlay only, no conflict"
            };
            println!("running: {} ({}) - {note}", c.tool, c.process);
        }
    }
    Ok(())
}

/// 64-bit FNV-1a. Stable across Rust versions, unlike `DefaultHasher`, so the same GPU hashes
/// the same in every report.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn windows_version() -> Option<String> {
    let out = std::process::Command::new("cmd").args(["/C", "ver"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[derive(Serialize)]
struct CompatRow {
    feature: Feature,
    verdict: Verdict,
}

#[derive(Serialize)]
struct DeviceReport {
    info: DeviceInfo,
    capabilities: Result<Capabilities, String>,
    telemetry: Result<Telemetry, String>,
    tuning_state: Result<TuningState, String>,
    compat: Vec<CompatRow>,
}

#[derive(Serialize)]
struct HardwareReport {
    report_schema: u32,
    tool_version: &'static str,
    generated_unix: u64,
    os: String,
    windows_version: Option<String>,
    backends: Vec<BackendStatus>,
    other_tools_running: Vec<Conflict>,
    devices: Vec<DeviceReport>,
}

/// A read-only hardware report for bug reports and the compatibility matrix. Device UUIDs are
/// replaced with a hash so a report doesn't fingerprint the machine.
pub fn doctor(simulate: Option<u32>, out: Option<std::path::PathBuf>) -> Result<()> {
    let opened = backends::open_all(simulate, 0.0);
    let matrix = CompatMatrix::builtin();
    let devices = opened
        .devices
        .iter()
        .map(|d| {
            let mut info = d.info().clone();
            let vendor_prefix = info.id.0.split(':').next().unwrap_or("gpu").to_string();
            info.id.0 = format!("{vendor_prefix}:{:016x}", fnv1a(&info.id.0));
            let compat = [Feature::Telemetry, Feature::PowerLimit, Feature::ClockOffset, Feature::FanControl]
                .into_iter()
                .map(|feature| CompatRow {
                    feature,
                    verdict: matrix.lookup(
                        info.vendor,
                        feature,
                        info.driver_version.as_deref(),
                        info.architecture.as_deref(),
                    ),
                })
                .collect();
            DeviceReport {
                info,
                capabilities: d.capabilities().map_err(|e| e.to_string()),
                telemetry: d.telemetry().map_err(|e| e.to_string()),
                tuning_state: d.tuning_state().map_err(|e| e.to_string()),
                compat,
            }
        })
        .collect();
    let report = HardwareReport {
        report_schema: 1,
        tool_version: env!("CARGO_PKG_VERSION"),
        generated_unix: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        windows_version: windows_version(),
        backends: opened.statuses,
        other_tools_running: conflicts::detect(),
        devices,
    };
    let text = serde_json::to_string_pretty(&report)?;
    match out {
        Some(path) => {
            std::fs::write(&path, &text)?;
            eprintln!("wrote {}", path.display());
        }
        None => println!("{text}"),
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct StepResult {
    feature: Feature,
    test: String,
    outcome: Outcome,
    restore: Option<Outcome>,
}

impl StepResult {
    fn passed(&self) -> bool {
        self.outcome.is_success() && self.restore.as_ref().is_none_or(Outcome::is_success)
    }
}

fn step(
    dev: &mut dyn GpuDevice,
    feature: Feature,
    test: Action,
    restore: Action,
    settle: Duration,
) -> StepResult {
    let label = test.to_string();
    // Fans need a moment to report the new intended speed on some boards.
    let outcome = if matches!(test, Action::SetFanDuty { .. }) {
        match dev.set_fan_duty(match test {
            Action::SetFanDuty { duty } => duty,
            _ => Percent::FULL,
        }) {
            Err(e) => Outcome::Failed { error: e.to_string() },
            Ok(()) => {
                std::thread::sleep(settle);
                tuner::write_and_verify(dev, &test)
            }
        }
    } else {
        tuner::write_and_verify(dev, &test)
    };
    // Always restore, whatever the test did.
    let restore = Some(tuner::write_and_verify(dev, &restore));
    StepResult { feature, test: label, outcome, restore }
}

pub fn selftest(
    simulate: Option<u32>,
    device: Option<String>,
    write: bool,
    ignore_conflicts: bool,
    json: bool,
) -> Result<()> {
    if !write {
        bail!(
            "selftest changes GPU settings (power limit -10 %, core +15 MHz, memory +50 MHz, fans 60 %) \
             for a few seconds each and restores them. Re-run with --write to confirm."
        );
    }
    if simulate.is_none() {
        let blocking: Vec<_> = conflicts::detect().into_iter().filter(Conflict::blocks_writes).collect();
        if !blocking.is_empty() && !ignore_conflicts {
            let names: Vec<_> = blocking.iter().map(|c| c.tool).collect();
            bail!(
                "close {} first: it would fight the test over the same settings and the results would be \
                 meaningless (or pass --ignore-conflicts)",
                names.join(", ")
            );
        }
    }
    let mut opened = backends::open_all(simulate, 0.0);
    let idx = match &device {
        Some(id) => opened.devices.iter().position(|d| d.info().id.0 == *id),
        None if opened.devices.len() == 1 => Some(0),
        None => bail!("several GPUs found; pick one with --device (see `sol probe`)"),
    };
    let Some(idx) = idx else { bail!("no such device; see `sol probe`") };
    let dev = opened.devices[idx].as_mut();
    let info = dev.info().clone();
    let caps = dev.capabilities()?;
    let before = dev.tuning_state()?;
    let settle = if info.vendor == Vendor::Simulated { Duration::ZERO } else { Duration::from_secs(3) };
    let mut results = Vec::new();

    if let (Some(pl), Some(orig)) = (caps.power_limit, before.power_limit) {
        let mut target = Milliwatts(((pl.default.0 as f64) * 0.90) as u32).clamp(pl.min, pl.max);
        if target == orig {
            target = Milliwatts(((pl.default.0 as f64) * 0.95) as u32).clamp(pl.min, pl.max);
        }
        results.push(step(
            dev,
            Feature::PowerLimit,
            Action::SetPowerLimit { limit: target },
            Action::SetPowerLimit { limit: orig },
            settle,
        ));
    }
    for (domain, delta) in [(ClockDomain::Core, 15), (ClockDomain::Memory, 50)] {
        if let (Some(range), Some(orig)) = (caps.offset(domain), before.offset(domain)) {
            let up = MhzOffset(orig.0 + delta);
            let target = if range.contains(up) { up } else { MhzOffset(orig.0 - delta) };
            results.push(step(
                dev,
                Feature::ClockOffset,
                Action::SetClockOffset { domain, offset: target },
                Action::SetClockOffset { domain, offset: orig },
                settle,
            ));
        }
    }
    if let Some(fc) = caps.fans {
        let current =
            dev.telemetry().ok().and_then(|t| t.fans.first().and_then(|f| f.duty)).map_or(0.0, Percent::get);
        let want = if (current - 60.0).abs() < 10.0 { 75.0 } else { 60.0 };
        results.push(step(
            dev,
            Feature::FanControl,
            Action::SetFanDuty { duty: fc.clamp(Percent::clamped(want)) },
            Action::FansAuto,
            settle,
        ));
    }

    let after = dev.tuning_state()?;
    let restored = after == before;

    if json {
        #[derive(Serialize)]
        struct Out<'a> {
            device: &'a DeviceInfo,
            results: &'a [StepResult],
            restored_to_original: bool,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&Out {
                device: &info,
                results: &results,
                restored_to_original: restored
            })?
        );
    } else {
        println!(
            "{} ({}, driver {})",
            info.name,
            info.architecture.as_deref().unwrap_or("?"),
            info.driver_version.as_deref().unwrap_or("?")
        );
        for r in &results {
            println!(
                "  {:<4} {:<13} {:<28} {:?}",
                if r.passed() { "PASS" } else { "FAIL" },
                r.feature.to_string(),
                r.test,
                r.outcome
            );
            if let Some(rest) = r.restore.as_ref().filter(|o| !o.is_success()) {
                println!("       restore failed: {rest:?}");
            }
        }
        println!(
            "  settings restored to original: {}",
            if restored { "yes" } else { "NO - check with `sol probe`" }
        );
        println!(
            "\nCompatibility matrix entries for these results (add the hardware report link as evidence):\n"
        );
        let mut features: Vec<Feature> = results.iter().map(|r| r.feature).collect();
        features.dedup();
        for feature in features {
            let rows: Vec<&StepResult> = results.iter().filter(|r| r.feature == feature).collect();
            let status = if rows.iter().all(|r| r.passed()) { "verified" } else { "broken" };
            let detail: Vec<String> = rows.iter().map(|r| format!("{}: {:?}", r.test, r.outcome)).collect();
            println!("[[entry]]");
            println!("vendor = \"{}\"", serde_json::to_value(info.vendor)?.as_str().unwrap_or("?"));
            println!("feature = \"{feature}\"");
            println!("status = \"{status}\"");
            if let Some(d) = &info.driver_version {
                println!("min_driver = \"{d}\"\nmax_driver = \"{d}\"");
            }
            if let Some(a) = &info.architecture {
                println!("architecture = \"{a}\"");
            }
            println!("note = \"sol selftest on {}: {}\"", info.name, detail.join("; ").replace('"', "'"));
            println!("evidence = \"<link to your hardware report issue>\"\n");
        }
    }
    if !restored || results.iter().any(|r| !r.passed()) {
        std::process::exit(2);
    }
    Ok(())
}
