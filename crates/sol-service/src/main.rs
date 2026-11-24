//! `sol-service run`: the long-running process that owns the GPUs.
//!
//! For now it runs in a console (elevated, for writes). Registration as a Windows service is
//! on the roadmap. The control loop works the same either way.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sol_core::compat::CompatMatrix;
use sol_core::tuner::SafetyPolicy;
use sol_service::{Service, ServiceConfig, backends, conflicts};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[derive(Parser, Debug)]
#[command(name = "sol-service", version, about = "Speed of Light GPU service")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the service in this console until Ctrl+C.
    Run {
        /// Use N simulated GPUs instead of real hardware.
        #[arg(long, value_name = "N")]
        simulate: Option<u32>,
        /// Load (0..1) for simulated GPUs.
        #[arg(long, default_value_t = 0.6)]
        sim_load: f32,
        /// Where the crash guard and other state live. Default: %ProgramData%\SpeedOfLight.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        #[arg(long, default_value = sol_ipc::PIPE_NAME)]
        pipe: String,
        /// Control-loop period in milliseconds (250..=5000).
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(250..=5000))]
        tick_ms: u64,
        /// Profile to apply at start (skipped if the crash guard tripped).
        #[arg(long)]
        startup_profile: Option<PathBuf>,
        /// Seconds an applied profile must survive before the crash guard disarms.
        #[arg(long, default_value_t = 120)]
        stability_secs: u64,
        /// Allow writes on driver/architecture combinations nobody has verified.
        #[arg(long)]
        allow_unverified: bool,
    },
}

fn default_state_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("SpeedOfLight")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let Command::Run {
        simulate,
        sim_load,
        state_dir,
        pipe,
        tick_ms,
        startup_profile,
        stability_secs,
        allow_unverified,
    } = Cli::parse().command;

    let opened = backends::open_all(simulate, sim_load);
    for s in &opened.statuses {
        tracing::info!(backend = s.backend, status = s.status, "{}", s.detail);
    }
    if opened.devices.is_empty() {
        tracing::warn!("no controllable GPUs found; serving status only");
    }
    if simulate.is_none() {
        for c in conflicts::detect().iter().filter(|c| c.blocks_writes()) {
            tracing::warn!(tool = c.tool, process = %c.process, "another GPU tool is running; it will fight this service over the same settings");
        }
    }

    let cfg = ServiceConfig {
        state_dir: state_dir.unwrap_or_else(default_state_dir),
        policy: SafetyPolicy { allow_unverified, ..SafetyPolicy::default() },
        startup_profile,
        stability_window: Duration::from_secs(stability_secs),
    };
    let svc = Arc::new(Mutex::new(Service::new(opened.devices, cfg, CompatMatrix::builtin())));

    {
        let mut s = svc.lock().unwrap_or_else(|e| e.into_inner());
        match s.startup() {
            Ok(reports) => {
                for r in reports {
                    tracing::info!(device = %r.device, profile = %r.profile, applied = r.all_applied(), "startup profile");
                }
            }
            Err(e) => tracing::warn!("{e}"),
        }
    }

    let (stop_tx, stop_rx) = watch::channel(false);

    let control = {
        let svc = Arc::clone(&svc);
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(tick_ms));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last = Instant::now();
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = stop.changed() => break,
                }
                let now = Instant::now();
                let dt = now - last;
                last = now;
                let svc = Arc::clone(&svc);
                let _ = tokio::task::spawn_blocking(move || {
                    svc.lock().unwrap_or_else(|e| e.into_inner()).tick(dt);
                })
                .await;
            }
        })
    };

    let handler: sol_ipc::framing::Handler = {
        let svc = Arc::clone(&svc);
        Arc::new(move |req| svc.lock().unwrap_or_else(|e| e.into_inner()).handle(req))
    };
    let server = {
        let pipe = pipe.clone();
        tokio::spawn(async move { sol_ipc::pipe::serve(&pipe, handler, stop_rx).await })
    };
    tracing::info!(%pipe, "listening; Ctrl+C to stop");

    tokio::select! {
        r = tokio::signal::ctrl_c() => r.context("waiting for Ctrl+C")?,
        r = server => {
            // The server only returns early on an error (e.g. the pipe name is taken).
            r.context("server task")?.with_context(|| format!("serving {pipe}"))?;
        }
    }
    tracing::info!("stopping");
    let _ = stop_tx.send(true);
    let _ = control.await;
    svc.lock().unwrap_or_else(|e| e.into_inner()).shutdown();
    Ok(())
}
