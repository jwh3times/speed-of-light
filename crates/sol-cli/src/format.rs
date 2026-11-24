//! Human-readable output. Every command also has `--json` for scripts.

use sol_core::device::{Capabilities, DeviceInfo, Telemetry};
use sol_core::tuner::{ApplyReport, Outcome};
use sol_ipc::{DeviceStatus, FanModeView};
use std::fmt::Display;

fn opt<T: Display>(v: Option<T>) -> String {
    v.map_or_else(|| "n/a".into(), |v| v.to_string())
}

pub fn telemetry_line(name: &str, t: &Telemetry) -> String {
    let fans = if t.fans.is_empty() {
        "fans n/a".to_string()
    } else {
        let f = &t.fans[0];
        format!("fans {} {}", opt(f.duty), opt(f.rpm))
    };
    let limiters = if t.limiters.is_empty() {
        "-".to_string()
    } else {
        t.limiters.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
    };
    format!(
        "{name} | {} | core {} mem {} | {} / {} | gpu {} | {fans} | limit: {limiters}",
        opt(t.gpu_temp),
        opt(t.core_clock),
        opt(t.memory_clock),
        opt(t.power_draw),
        opt(t.power_limit),
        opt(t.gpu_utilization),
    )
}

pub fn device_block(info: &DeviceInfo, caps: Option<&Capabilities>) -> String {
    let mut s = format!(
        "{}  [{}]\n  vendor {}  arch {}  driver {}  vbios {}\n",
        info.name,
        info.id,
        info.vendor,
        opt(info.architecture.as_deref()),
        opt(info.driver_version.as_deref()),
        opt(info.vbios_version.as_deref()),
    );
    if let Some(c) = caps {
        s += &match &c.power_limit {
            Some(p) => format!("  power limit  {}..={} (default {})\n", p.min, p.max, p.default),
            None => "  power limit  not adjustable\n".into(),
        };
        for (label, o) in [("core offset ", &c.core_offset), ("mem offset  ", &c.memory_offset)] {
            s += &match o {
                Some(o) => format!("  {label} {}..={}\n", o.min, o.max),
                None => format!("  {label} not adjustable\n"),
            };
        }
        s += &match &c.fans {
            Some(f) => format!("  fans         {} (duty {}..={})\n", f.count, f.min_duty, f.max_duty),
            None => "  fans         not controllable\n".into(),
        };
        s += &format!("  slowdown {}  shutdown {}\n", opt(c.slowdown_temp), opt(c.shutdown_temp));
    }
    s
}

pub fn status_block(d: &DeviceStatus) -> String {
    let mut s = device_block(&d.info, d.capabilities.as_ref());
    let fan = match &d.fan_mode {
        FanModeView::Firmware => "firmware".to_string(),
        FanModeView::Fixed { duty } => format!("fixed {duty}"),
        FanModeView::Curve { last_duty } => format!("curve (now {})", opt(*last_duty)),
    };
    s += &format!("  profile {}  fan mode {fan}\n", opt(d.active_profile.as_deref()));
    match (&d.telemetry, &d.telemetry_error) {
        (_, Some(e)) => s += &format!("  telemetry error: {e}\n"),
        (Some(t), None) => s += &format!("  {}\n", telemetry_line("now", t)),
        (None, None) => s += "  no telemetry yet\n",
    }
    for n in &d.notices {
        s += &format!("  ! {n}\n");
    }
    s
}

pub fn report(r: &ApplyReport) -> String {
    let mut s = format!("{} <- {}\n", r.device, r.profile);
    for item in &r.items {
        let line = match &item.outcome {
            Outcome::Applied => format!("  ok          {}", item.item),
            Outcome::NotEnforced { expected, actual } => {
                format!(
                    "  NOT ENFORCED {}: driver accepted it but reads back {actual} (wanted {expected})",
                    item.item
                )
            }
            Outcome::Rejected { reason } => format!("  rejected    {}: {reason}", item.item),
            Outcome::Blocked { reason } => format!("  blocked     {}: {reason}", item.item),
            Outcome::Failed { error } => format!("  FAILED      {}: {error}", item.item),
        };
        s += &line;
        s.push('\n');
    }
    s
}
