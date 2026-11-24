use serde::{Deserialize, Serialize};
use sol_core::crashguard::Sentinel;
use sol_core::device::{Capabilities, DeviceId, DeviceInfo, Telemetry};
use sol_core::profile::{FanSetting, Profile};
use sol_core::tuner::ApplyReport;
use sol_core::units::Percent;

pub const PROTOCOL_VERSION: u32 = 1;

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
    Status { devices: Vec<DeviceStatus>, guard: Option<Sentinel> },
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
    }
}
