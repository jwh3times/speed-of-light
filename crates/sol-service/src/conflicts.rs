//! Detects other tools that write to the GPU. Two tools driving the same fans or clocks fight
//! each other: each re-applies its own value, and the user sees the fans "randomly" change.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Sets clocks, power or voltage, and usually fans too.
    Tuning,
    /// Drives fans.
    FanControl,
    /// On-screen display only. Doesn't conflict with tuning.
    Overlay,
}

#[derive(Debug, Clone, Serialize)]
pub struct Conflict {
    pub tool: &'static str,
    pub process: String,
    pub role: Role,
}

impl Conflict {
    pub fn blocks_writes(&self) -> bool {
        matches!(self.role, Role::Tuning | Role::FanControl)
    }
}

/// (lowercase process-name prefix, tool name, role)
const KNOWN: &[(&str, &str, Role)] = &[
    ("msiafterburner", "MSI Afterburner", Role::Tuning),
    ("gputweakiii", "ASUS GPU Tweak III", Role::Tuning),
    ("precisionx", "EVGA Precision X1", Role::Tuning),
    ("mvolt", "mVolt+", Role::Tuning),
    ("fancontrol", "Fan Control", Role::FanControl),
    ("argusmonitor", "Argus Monitor", Role::FanControl),
    ("rtss", "RivaTuner Statistics Server", Role::Overlay),
];

pub fn classify(process_names: impl IntoIterator<Item = String>) -> Vec<Conflict> {
    let mut found: Vec<Conflict> = Vec::new();
    for name in process_names {
        let lower = name.to_lowercase();
        if let Some(&(_, tool, role)) = KNOWN.iter().find(|(prefix, _, _)| lower.starts_with(prefix))
            && !found.iter().any(|c| c.tool == tool)
        {
            found.push(Conflict { tool, process: name, role });
        }
    }
    found
}

/// Scan running processes. Errors yield an empty list: this is advisory, never a gate on
/// reading telemetry.
pub fn detect() -> Vec<Conflict> {
    classify(process_names())
}

#[cfg(windows)]
fn process_names() -> Vec<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut names = Vec::new();
    // SAFETY: standard ToolHelp enumeration; `entry.dwSize` is set before use and the
    // snapshot handle is closed on every path.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return names;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                names.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    names
}

#[cfg(not(windows))]
fn process_names() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_tools_once() {
        let found = classify(
            ["explorer.exe", "MSIAfterburner.exe", "RTSS.exe", "RTSSHooksLoader64.exe", "FanControl.exe"]
                .map(String::from),
        );
        let tools: Vec<_> = found.iter().map(|c| (c.tool, c.blocks_writes())).collect();
        assert_eq!(
            tools,
            vec![("MSI Afterburner", true), ("RivaTuner Statistics Server", false), ("Fan Control", true)]
        );
    }

    #[test]
    fn detect_runs() {
        // Smoke test: enumeration must not crash. What it finds depends on the machine.
        let _ = detect();
    }
}
