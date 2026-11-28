//! The Windows service registration as plain data. `install` hands this to the Service
//! Control Manager; building it here, without Win32, lets tests check every field.

use crate::cli::Simulation;
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
/// The administrator-owned copy of the service config, inside the state folder.
pub const CONFIG_FILE: &str = "config.toml";
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

/// The service's settings are in its service config; only the simulation is on its command line.
pub fn registration(program_files: &Path, sim: &Simulation) -> Registration {
    let mut arguments = vec!["service".to_string()];
    arguments.extend(sim.to_args());
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
///
/// It reads the tokens rather than parsing them, so a registration written by an earlier
/// version, whose `service` command line still carried the settings flags, is read the same way.
pub fn uninstall_resets_real_gpus(stored_command_line: Option<&str>) -> bool {
    let simulated = stored_command_line.map(arguments_from_command_line).is_some_and(|args| {
        args.first().is_some_and(|a| a == "service")
            && args.iter().any(|a| a == "--simulate" || a.starts_with("--simulate="))
    });
    !simulated
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    /// What the `service` subcommand makes of these arguments, as the SCM passes them.
    fn service_parses(arguments: &[String]) -> Simulation {
        let argv = std::iter::once("sol-service").chain(arguments.iter().map(String::as_str));
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Service { sim } => sim,
            other => panic!("{other:?}"),
        }
    }

    fn sim(args: &[&str]) -> Simulation {
        let argv: Vec<String> =
            std::iter::once("service").chain(args.iter().copied()).map(String::from).collect();
        service_parses(&argv)
    }

    #[test]
    fn the_service_runs_as_local_system_at_boot_from_program_files() {
        let reg = registration(Path::new(r"C:\Program Files"), &sim(&[]));
        assert_eq!(reg.name, "SpeedOfLight");
        assert_eq!(reg.display_name, "Speed of Light");
        assert_eq!(reg.executable, PathBuf::from(r"C:\Program Files\SpeedOfLight\sol-service.exe"));
        assert_eq!(reg.account, Account::LocalSystem);
        assert_eq!(reg.start, StartType::Automatic);
        assert_eq!(reg.arguments.first().map(String::as_str), Some("service"));
    }

    #[test]
    fn a_dead_service_is_restarted_within_seconds_every_time() {
        let reg = registration(Path::new(r"C:\Program Files"), &sim(&[]));
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
    fn the_registered_command_line_carries_no_settings_only_the_simulation() {
        // The settings live in the service config; the command line can't say what it holds.
        let real = registration(Path::new(r"C:\Program Files"), &sim(&[]));
        assert_eq!(real.arguments, ["service"]);
        let simulated =
            registration(Path::new(r"C:\Program Files"), &sim(&["--simulate", "2", "--sim-load", "0.25"]));
        assert_eq!(simulated.arguments, ["service", "--simulate", "2", "--sim-load", "0.25"]);
        // The service starts with the simulation install was given.
        for given in [sim(&[]), sim(&["--simulate", "2", "--sim-load", "0.25"])] {
            let reg = registration(Path::new(r"C:\Program Files"), &given);
            assert_eq!(service_parses(&reg.arguments), given, "{:?}", reg.arguments);
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
        let simulated =
            r#""C:\Program Files\SpeedOfLight\sol-service.exe" service --simulate 1 --sim-load 0.6"#;
        let real = r#""C:\Program Files\SpeedOfLight\sol-service.exe" service"#;
        assert!(!uninstall_resets_real_gpus(Some(simulated)));
        assert!(uninstall_resets_real_gpus(Some(real)));
        // A registration from before the service config (v0.2) still carries its settings
        // flags. This version's uninstall must read it the same way.
        let old_simulated = r#""C:\Program Files\SpeedOfLight\sol-service.exe" service --tick-ms 1000 --stability-secs 120 --log-level info --allow-unverified --simulate 1 --sim-load 0.6"#;
        let old_real = r#""C:\Program Files\SpeedOfLight\sol-service.exe" service --tick-ms 1000 --stability-secs 120 --log-level info"#;
        assert!(!uninstall_resets_real_gpus(Some(old_simulated)));
        assert!(uninstall_resets_real_gpus(Some(old_real)));
        // The flag's `=` form, which the parser accepts too.
        let equals_form = r#""C:\Program Files\SpeedOfLight\sol-service.exe" service --simulate=1"#;
        assert!(!uninstall_resets_real_gpus(Some(equals_form)));
        // `--simulate` only counts in a `service` command line.
        assert!(uninstall_resets_real_gpus(Some(r"C:\sol\sol-service.exe run --simulate 1")));
        // When the stored command line can't be read or understood, uninstall still does its
        // job: returning real GPUs to stock.
        assert!(uninstall_resets_real_gpus(None));
        assert!(uninstall_resets_real_gpus(Some("something else entirely")));
    }
}
