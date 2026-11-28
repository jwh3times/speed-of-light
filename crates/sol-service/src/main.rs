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
use sol_service::config::{self, ServiceConfig};
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
        Command::Run { sim, config, overrides, state_dir, pipe, startup_profile } => {
            // In a console the user named the files themselves, so they aren't trust-checked,
            // and a config that can't be used stops the run instead of blocking writes.
            let file = match &config {
                None => ServiceConfig::default(),
                Some(p) => config::read_named(p).map_err(anyhow::Error::msg)?.1,
            };
            let config = overrides.over(file);
            init_console_logging(config.log_level);
            if sim.simulate.is_none()
                && let Err(why) = winsvc::console_may_open_real_gpus(&winsvc::installed_state())
            {
                bail!("{why}");
            }
            let cfg = HostConfig {
                sim,
                config,
                config_rejected: None,
                state_dir: state_dir.unwrap_or_else(host::default_state_dir),
                pipe,
                startup_profile: startup_profile.map(|p| host::load_startup_profile(&p, false)),
            };
            tracing::info!("Ctrl+C to stop");
            host::run(cfg, async {
                if let Err(e) = tokio::signal::ctrl_c().await {
                    tracing::error!("waiting for Ctrl+C: {e}");
                }
            })
            .await
        }
        Command::Install { sim, config, startup_profile } => {
            winsvc::install(&sim, config.as_deref(), startup_profile.as_deref())
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
        Command::Service { sim } => tokio::task::block_in_place(|| winsvc::run_dispatcher(sim)),
    }
}
