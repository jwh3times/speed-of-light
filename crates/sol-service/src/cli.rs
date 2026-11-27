//! `sol-service`'s command line. It lives in the library so the arguments `install` stores in
//! the service's registration are parsed back by the very parser the service starts with.

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
        settings: Settings,
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
        settings: Settings,
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
        settings: Settings,
    },
}

/// Flags shared by `run` and the installed service. `install` stores them in the service's
/// command line.
#[derive(Args, Debug, Clone, PartialEq)]
pub struct Settings {
    /// Use N simulated GPUs instead of real hardware (for testing; touches no GPU).
    #[arg(long, value_name = "N")]
    pub simulate: Option<u32>,
    /// Load (0..1) for simulated GPUs.
    #[arg(long, default_value_t = 0.6)]
    pub sim_load: f32,
    /// Control-loop period in milliseconds (250..=5000).
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(250..=5000))]
    pub tick_ms: u64,
    /// Seconds an applied profile must survive before the crash guard disarms.
    #[arg(long, default_value_t = 120)]
    pub stability_secs: u64,
    /// Allow writes on driver/architecture combinations nobody has verified.
    #[arg(long)]
    pub allow_unverified: bool,
    /// error, warn, info, debug or trace.
    #[arg(long, default_value_t = tracing::Level::INFO)]
    pub log_level: tracing::Level,
}

impl Settings {
    /// These settings as command-line arguments, as the service's registration stores them.
    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            "--tick-ms".to_string(),
            self.tick_ms.to_string(),
            "--stability-secs".to_string(),
            self.stability_secs.to_string(),
            "--log-level".to_string(),
            self.log_level.to_string().to_lowercase(),
        ];
        if self.allow_unverified {
            args.push("--allow-unverified".into());
        }
        if let Some(n) = self.simulate {
            args.extend(["--simulate".into(), n.to_string(), "--sim-load".into(), self.sim_load.to_string()]);
        }
        args
    }
}
