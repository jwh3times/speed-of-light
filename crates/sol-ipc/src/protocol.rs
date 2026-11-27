use serde::{Deserialize, Serialize};
use sol_core::crashguard::Sentinel;
use sol_core::device::{Capabilities, DeviceId, DeviceInfo, Telemetry};
use sol_core::profile::{FanSetting, Profile};
use sol_core::tuner::ApplyReport;
use sol_core::units::Percent;

/// 2: `status` carries `backends`, so a client can tell "waiting for the driver" from "no GPU".
pub const PROTOCOL_VERSION: u32 = 2;

/// What opening one vendor backend found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendState {
    Ok,
    /// The vendor's runtime isn't installed.
    NotPresent,
    /// The runtime is installed but not usable yet (the driver is still loading). Retried.
    NotReady,
    /// The runtime is installed but this backend can't use it (not implemented).
    Unavailable,
    Error,
}

impl std::fmt::Display for BackendState {
    /// The wire name, e.g. `not_ready`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BackendState::Ok => "ok",
            BackendState::NotPresent => "not_present",
            BackendState::NotReady => "not_ready",
            BackendState::Unavailable => "unavailable",
            BackendState::Error => "error",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendStatus {
    pub backend: String,
    pub status: BackendState,
    pub detail: String,
    pub devices: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        protocol: u32,
    },
    Status,
    ApplyProfile {
        profile: Profile,
        #[serde(default)]
        allow_unverified: bool,
    },
    /// Return devices to stock: default power limit, zero offsets, firmware fans.
    Reset {
        device: Option<DeviceId>,
    },
    SetFan {
        device: Option<DeviceId>,
        setting: FanSetting,
        #[serde(default)]
        allow_unverified: bool,
    },
    GuardStatus,
    GuardClear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello { protocol: u32, service_version: String },
    Status { devices: Vec<DeviceStatus>, guard: Option<Sentinel>, backends: Vec<BackendStatus> },
    Applied { reports: Vec<ApplyReport> },
    Guard { tripped: Option<Sentinel> },
    Ok { message: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum FanModeView {
    Firmware,
    Fixed { duty: Percent },
    Curve { last_duty: Option<Percent> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub info: DeviceInfo,
    pub capabilities: Option<Capabilities>,
    pub telemetry: Option<Telemetry>,
    pub telemetry_error: Option<String>,
    pub fan_mode: FanModeView,
    pub active_profile: Option<String>,
    /// Things the service did on its own, newest last (e.g. "fans returned to firmware").
    pub notices: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_as_tagged_json() {
        let profile = Profile::from_toml(
            "schema = 1\nname = \"p\"\n[fan]\nmode = \"curve\"\npoints = [[40, 30], [80, 100]]\n",
        )
        .unwrap();
        let reqs = vec![
            Request::Hello { protocol: PROTOCOL_VERSION },
            Request::Status,
            Request::ApplyProfile { profile, allow_unverified: true },
            Request::Reset { device: Some(DeviceId("sim:0".into())) },
            Request::SetFan {
                device: None,
                setting: FanSetting::Fixed { duty: Percent::new(40.0).unwrap() },
                allow_unverified: false,
            },
            Request::GuardClear,
        ];
        for r in reqs {
            let json = serde_json::to_string(&r).unwrap();
            assert!(!json.contains('\n'));
            assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), r, "{json}");
        }
    }

    #[test]
    fn wire_format_is_stable() {
        let json = serde_json::to_string(&Request::Reset { device: None }).unwrap();
        assert_eq!(json, r#"{"type":"reset","device":null}"#);
        let r: Request =
            serde_json::from_str(r#"{"type":"apply_profile","profile":{"schema":1,"name":"x"}}"#).unwrap();
        assert!(matches!(r, Request::ApplyProfile { allow_unverified: false, .. }));

        assert_eq!(PROTOCOL_VERSION, 2, "protocol 2 added backend statuses to `status`");
        let status = Response::Status {
            devices: Vec::new(),
            guard: None,
            backends: vec![BackendStatus {
                backend: "nvidia-nvml".into(),
                status: BackendState::NotReady,
                detail: "NVIDIA driver is not loaded".into(),
                devices: 0,
            }],
        };
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            r#"{"type":"status","devices":[],"guard":null,"backends":[{"backend":"nvidia-nvml","status":"not_ready","detail":"NVIDIA driver is not loaded","devices":0}]}"#
        );
    }
}
