//! `sol-service`: the process that owns the GPUs.
//!
//! - `run`: in this console until Ctrl+C (development, `--simulate`).
//! - `install` / `uninstall`: register or remove the Windows service (elevated).
//! - `service`: what the Service Control Manager starts.
//!
//! Both `run` and the service run [`sol_service::host::run`]; see `docs/architecture.md`.

use anyhow::{Result, bail};
use clap::Parser;
use sol_service::cli::{Cli, Command};
use sol_service::host::{self, HostConfig};
use sol_service::winsvc;

fn init_console_logging(level: tracing::Level) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level.to_string().to_lowercase()));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run { settings, state_dir, pipe, startup_profile } => {
            init_console_logging(settings.log_level);
            if settings.simulate.is_none()
                && let Err(why) = winsvc::console_may_open_real_gpus(&winsvc::installed_state())
            {
                bail!("{why}");
            }
            let cfg = HostConfig {
                state_dir: state_dir.unwrap_or_else(host::default_state_dir),
                pipe,
                // In a console the user named the file themselves, so it isn't trust-checked.
                startup_profile: startup_profile.map(|p| host::load_startup_profile(&p, false)),
                settings,
            };
            tracing::info!("Ctrl+C to stop");
            host::run(cfg, async {
                if let Err(e) = tokio::signal::ctrl_c().await {
                    tracing::error!("waiting for Ctrl+C: {e}");
                }
            })
            .await
        }
        Command::Install { settings, startup_profile } => {
            winsvc::install(&settings, startup_profile.as_deref())
        }
        Command::Uninstall { purge } => {
            let code = winsvc::uninstall(purge)?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        // The dispatcher blocks this thread until the service stops; the service builds its own
        // runtime, so leave this one alone.
        Command::Service { settings } => tokio::task::block_in_place(|| winsvc::run_dispatcher(settings)),
    }
}
