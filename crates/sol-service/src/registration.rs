//! The Windows service registration as plain data. `install` hands this to the Service
//! Control Manager; building it here, without Win32, lets tests check every field.

use crate::cli::{Cli, Command, Settings};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// May change with the branding decision (#21): keep every use going through this constant.
pub const SERVICE_NAME: &str = "SpeedOfLight";
pub const DISPLAY_NAME: &str = "Speed of Light";
pub const DESCRIPTION: &str =
    "Owns the GPUs for Speed of Light: fan curves, power and clock settings, and monitoring.";
pub const EXE_NAME: &str = "sol-service.exe";
/// The administrator-owned copy of the startup profile, inside the state folder.
pub const STARTUP_PROFILE_FILE: &str = "startup-profile.toml";
pub const LOG_DIR: &str = "logs";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Account {
    /// NVML's set calls need administrator rights; no unprivileged service account has them.
    LocalSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartType {
    /// At boot, not delayed. The service waits for the GPU driver itself.
    Automatic,
}

/// What the Service Control Manager does when the service dies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// Restart after each delay in turn; the last one repeats for every later failure.
    pub restart_after: Vec<Duration>,
    /// Forget earlier failures after this long without one.
    pub reset_after: Duration,
    /// Also recover when the service stops with an error, not only when it is killed.
    pub on_error_exit: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Registration {
    pub name: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    /// The administrator-only copy under Program Files, never where `install` ran from.
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub account: Account,
    pub start: StartType,
    pub recovery: Recovery,
}

/// `%ProgramFiles%\SpeedOfLight`.
pub fn install_dir(program_files: &Path) -> PathBuf {
    program_files.join(SERVICE_NAME)
}

/// `%ProgramData%\SpeedOfLight`.
pub fn state_dir(program_data: &Path) -> PathBuf {
    program_data.join(SERVICE_NAME)
}

pub fn registration(program_files: &Path, settings: &Settings) -> Registration {
    let mut arguments = vec!["service".to_string()];
    arguments.extend(settings.to_args());
    Registration {
        name: SERVICE_NAME,
        display_name: DISPLAY_NAME,
        description: DESCRIPTION,
        executable: install_dir(program_files).join(EXE_NAME),
        arguments,
        account: Account::LocalSystem,
        start: StartType::Automatic,
        recovery: Recovery {
            restart_after: vec![Duration::from_secs(2), Duration::from_secs(2), Duration::from_secs(10)],
            reset_after: Duration::from_secs(24 * 60 * 60),
            on_error_exit: true,
        },
    }
}

/// The arguments in a stored service command line, without the executable. Our arguments
/// never contain spaces, so quotes only ever wrap whole tokens.
pub fn arguments_from_command_line(command_line: &str) -> Vec<String> {
    let line = command_line.trim_start();
    let rest = match line.strip_prefix('"') {
        Some(quoted) => quoted.split_once('"').map_or("", |(_, rest)| rest),
        None => line.split_once(char::is_whitespace).map_or("", |(_, rest)| rest),
    };
    rest.split_whitespace().map(|t| t.trim_matches('"').to_string()).collect()
}

/// Whether uninstall should return the real GPUs to stock, given the service's stored command
/// line. Only an install that provably used `--simulate` is spared: it never touched a GPU.
pub fn uninstall_resets_real_gpus(stored_command_line: Option<&str>) -> bool {
    let simulated = stored_command_line
        .and_then(|line| settings_from_arguments(&arguments_from_command_line(line)))
        .is_some_and(|s| s.simulate.is_some());
    !simulated
}

/// The settings a registered service starts with, from its stored arguments. `None` when
/// they aren't a `service` command line this version understands.
pub fn settings_from_arguments(arguments: &[String]) -> Option<Settings> {
    let argv = std::iter::once("sol-service").chain(arguments.iter().map(String::as_str));
    match Cli::try_parse_from(argv).ok()?.command {
        Command::Service { settings } => Some(settings),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn settings(args: &[&str]) -> Settings {
        let argv = ["sol-service", "service"].iter().chain(args).copied();
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Service { settings } => settings,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_service_runs_as_local_system_at_boot_from_program_files() {
        let reg = registration(Path::new(r"C:\Program Files"), &settings(&[]));
        assert_eq!(reg.name, "SpeedOfLight");
        assert_eq!(reg.display_name, "Speed of Light");
        assert_eq!(reg.executable, PathBuf::from(r"C:\Program Files\SpeedOfLight\sol-service.exe"));
        assert_eq!(reg.account, Account::LocalSystem);
        assert_eq!(reg.start, StartType::Automatic);
        assert_eq!(reg.arguments.first().map(String::as_str), Some("service"));
    }

    #[test]
    fn a_dead_service_is_restarted_within_seconds_every_time() {
        let reg = registration(Path::new(r"C:\Program Files"), &settings(&[]));
        assert_eq!(
            reg.recovery,
            Recovery {
                restart_after: vec![Duration::from_secs(2), Duration::from_secs(2), Duration::from_secs(10)],
                reset_after: Duration::from_secs(24 * 60 * 60),
                on_error_exit: true,
            }
        );
    }

    #[test]
    fn stored_arguments_start_the_service_with_the_settings_install_was_given() {
        for args in [
            &[][..],
            &["--allow-unverified", "--tick-ms", "500", "--stability-secs", "300"][..],
            &["--simulate", "2", "--sim-load", "0.25", "--log-level", "debug"][..],
            &["--log-level", "warn", "--tick-ms", "5000"][..],
        ] {
            let given = settings(args);
            let reg = registration(Path::new(r"C:\Program Files"), &given);
            assert_eq!(
                settings_from_arguments(&reg.arguments),
                Some(given),
                "{args:?} -> {:?}",
                reg.arguments
            );
        }
    }

    #[test]
    fn the_command_line_windows_stores_splits_back_into_the_arguments() {
        // The Service Control Manager keeps one string: the quoted path, then the arguments.
        let stored =
            r#""C:\Program Files\SpeedOfLight\sol-service.exe" service --tick-ms 500 --log-level info"#;
        assert_eq!(
            arguments_from_command_line(stored),
            ["service", "--tick-ms", "500", "--log-level", "info"]
        );
        assert_eq!(arguments_from_command_line(r"C:\sol\sol-service.exe service"), ["service"]);
        assert_eq!(
            arguments_from_command_line(r#""C:\a b\sol-service.exe" service "--sim-load" 0.5"#),
            ["service", "--sim-load", "0.5"]
        );
        assert!(arguments_from_command_line("").is_empty());
        // An executable with no arguments, and a path whose closing quote is missing.
        assert!(arguments_from_command_line(r"C:\sol\sol-service.exe").is_empty());
        assert!(arguments_from_command_line(r#""C:\a b\sol-service.exe service --simulate 1"#).is_empty());
    }

    #[test]
    fn uninstalling_a_simulated_install_touches_no_real_gpu() {
        let simulated = r#""C:\Program Files\SpeedOfLight\sol-service.exe" service --tick-ms 1000 --simulate 1 --sim-load 0.6"#;
        let real = r#""C:\Program Files\SpeedOfLight\sol-service.exe" service --tick-ms 1000"#;
        assert!(!uninstall_resets_real_gpus(Some(simulated)));
        assert!(uninstall_resets_real_gpus(Some(real)));
        // When the stored command line can't be read or understood, uninstall still does its
        // job: returning real GPUs to stock.
        assert!(uninstall_resets_real_gpus(None));
        assert!(uninstall_resets_real_gpus(Some("something else entirely")));
    }

    #[test]
    fn arguments_that_are_not_a_service_command_line_are_not_understood() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(settings_from_arguments(&args(&["run", "--simulate", "1"])), None);
        assert_eq!(settings_from_arguments(&args(&["service", "--no-such-flag"])), None);
        assert_eq!(settings_from_arguments(&[]), None);
    }
}
