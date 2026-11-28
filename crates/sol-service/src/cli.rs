//! `sol-service`'s command line. It lives in the library so the arguments `install` stores in
//! the service's registration are parsed back by the very parser the service starts with.
//!
//! The installed service takes its settings from its service config (`install --config`), not
//! from flags: only the simulate flags reach its command line. A console `run` takes the same
//! settings as flags, which win over a `--config` file for their key.

use crate::config::{self, ServiceConfig};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "sol-service", version, about = "Speed of Light GPU service")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
    /// Run the service in this console until Ctrl+C (development, `--simulate`).
    Run {
        #[command(flatten)]
        sim: Simulation,
        /// A service config to read in place. Flags given here win over it.
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(flatten)]
        overrides: Overrides,
        /// Where the crash guard and other state live. Default: %ProgramData%\SpeedOfLight.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        #[arg(long, default_value = sol_ipc::PIPE_NAME)]
        pipe: String,
        /// Profile to apply once a GPU is found (skipped if the crash guard tripped).
        #[arg(long)]
        startup_profile: Option<PathBuf>,
    },
    /// Install or upgrade the Windows service, and start it. Needs an elevated terminal.
    Install {
        #[command(flatten)]
        sim: Simulation,
        /// The service config: policy caps, tick rate, stability window, log level. It is
        /// checked, then copied into the administrator-only state folder; editing the original
        /// later changes nothing. Without it the service runs on the defaults.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Profile the service applies at every start. It is checked, then copied into the
        /// administrator-only state folder; editing the original later changes nothing.
        #[arg(long)]
        startup_profile: Option<PathBuf>,
    },
    /// Stop and remove the Windows service, and return every GPU to stock.
    Uninstall {
        /// Also delete the logs and the crash guard (the whole state folder).
        #[arg(long)]
        purge: bool,
    },
    /// What the Service Control Manager starts. Not for use from a terminal.
    #[command(hide = true)]
    Service {
        #[command(flatten)]
        sim: Simulation,
    },
}

/// Simulated GPUs instead of real ones. `install` stores these in the service's command line.
#[derive(Args, Debug, Clone, PartialEq)]
pub struct Simulation {
    /// Use N simulated GPUs instead of real hardware (for testing; touches no GPU).
    #[arg(long, value_name = "N")]
    pub simulate: Option<u32>,
    /// Load (0..1) for simulated GPUs.
    #[arg(long, default_value_t = 0.6)]
    pub sim_load: f32,
}

impl Simulation {
    /// These flags as command-line arguments, as the service's registration stores them.
    pub fn to_args(&self) -> Vec<String> {
        match self.simulate {
            None => Vec::new(),
            Some(n) => {
                vec!["--simulate".into(), n.to_string(), "--sim-load".into(), self.sim_load.to_string()]
            }
        }
    }
}

/// A console run's settings flags. Each one given wins over the service config for its key.
#[derive(Args, Debug, Clone, PartialEq)]
pub struct Overrides {
    /// Control-loop period in milliseconds (250..=5000). Default: 1000.
    #[arg(long, value_parser = clap::value_parser!(u64).range(config::TICK_MS))]
    pub tick_ms: Option<u64>,
    /// Seconds an applied profile must survive before the crash guard disarms (30..=3600).
    /// Default: 120.
    #[arg(long, value_parser = clap::value_parser!(u64).range(config::STABILITY_SECS))]
    pub stability_secs: Option<u64>,
    /// Allow writes on driver/architecture combinations nobody has verified.
    #[arg(long)]
    pub allow_unverified: bool,
    /// error, warn, info, debug or trace. Default: info.
    #[arg(long, value_parser = parse_log_level)]
    pub log_level: Option<tracing::Level>,
}

fn parse_log_level(name: &str) -> Result<tracing::Level, String> {
    config::log_level_named(name).ok_or_else(|| "expected error, warn, info, debug or trace".into())
}

impl Overrides {
    /// `base` with each flag that was given put in place of its value.
    pub fn over(self, mut base: ServiceConfig) -> ServiceConfig {
        if let Some(t) = self.tick_ms {
            base.tick_ms = t;
        }
        if let Some(s) = self.stability_secs {
            base.stability_secs = s;
        }
        if self.allow_unverified {
            base.policy.allow_unverified = true;
        }
        if let Some(l) = self.log_level {
            base.log_level = l;
        }
        base
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    fn run(args: &[&str]) -> (Option<PathBuf>, Overrides) {
        let argv = ["sol-service", "run"].iter().chain(args).copied();
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Run { config, overrides, .. } => (config, overrides),
            other => panic!("{other:?}"),
        }
    }

    fn from_file() -> ServiceConfig {
        ServiceConfig::from_toml("schema = 1\ntick_ms = 2000\nstability_secs = 300\nlog_level = \"warn\"\n")
            .unwrap()
    }

    #[test]
    fn a_console_flag_wins_over_the_config_file_for_its_key_only() {
        let (config, overrides) = run(&["--config", "my.toml", "--tick-ms", "500", "--allow-unverified"]);
        assert_eq!(config, Some(PathBuf::from("my.toml")));
        let cfg = overrides.over(from_file());
        assert_eq!(cfg.tick_ms, 500);
        assert!(cfg.policy.allow_unverified);
        assert_eq!((cfg.stability_secs, cfg.log_level), (300, tracing::Level::WARN));
    }

    #[test]
    fn without_flags_a_console_run_uses_the_file_or_the_defaults() {
        let (config, overrides) = run(&[]);
        assert_eq!(config, None);
        assert_eq!(overrides.clone().over(from_file()), from_file());
        assert_eq!(overrides.over(ServiceConfig::default()), ServiceConfig::default());
    }

    #[test]
    fn the_stability_window_flag_takes_the_config_range() {
        let parse = |secs: &str| Cli::try_parse_from(["sol-service", "run", "--stability-secs", secs]);
        assert!(parse("29").is_err() && parse("3601").is_err());
        assert!(parse("30").is_ok() && parse("3600").is_ok());
    }

    #[test]
    fn install_and_the_service_take_no_setting_flags() {
        // Each flag as `run` accepts it, so a refusal can only mean the command lacks the flag.
        let flags: [&[&str]; 4] = [
            &["--tick-ms", "500"],
            &["--stability-secs", "300"],
            &["--log-level", "debug"],
            &["--allow-unverified"],
        ];
        for flag in flags {
            let with = |cmd: &str| Cli::try_parse_from(["sol-service", cmd].iter().chain(flag).copied());
            assert!(with("run").is_ok(), "run refused {flag:?}");
            for cmd in ["install", "service"] {
                assert!(with(cmd).is_err(), "{cmd} accepted {flag:?}");
            }
        }
        let parsed = Cli::try_parse_from(["sol-service", "install", "--config", "c.toml", "--simulate", "1"]);
        let Command::Install { config, sim, .. } = parsed.unwrap().command else { panic!() };
        assert_eq!((config, sim.simulate), (Some(PathBuf::from("c.toml")), Some(1)));
    }
}
