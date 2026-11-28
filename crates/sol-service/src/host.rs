//! The long-running host around [`Service`]: the tokio control loop and the named pipe. The
//! console (`sol-service run`) and the Windows service share it, and differ only in how they
//! are asked to stop.

use crate::cli::Simulation;
use crate::config::{self, ServiceConfig};
use crate::{Service, ServiceOptions, StartupProfile, backends, conflicts};
use anyhow::{Context, Result};
use sol_core::compat::CompatMatrix;
use sol_core::profile::Profile;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub sim: Simulation,
    /// The settings the service runs with: the service config's, or the defaults.
    pub config: ServiceConfig,
    /// Why the service config was rejected; the service then blocks writes (ADR 0003).
    pub config_rejected: Option<String>,
    pub state_dir: PathBuf,
    pub pipe: String,
    pub startup_profile: Option<StartupProfile>,
}

/// Load the installed service's config: a file a non-administrator could have changed is
/// rejected without being read.
pub fn load_service_config(path: &Path) -> config::Loaded {
    config::load_installed(path, |p| crate::trust::untrusted_reason(p).map_err(|e| e.to_string()))
}

/// `%ProgramData%\SpeedOfLight`.
pub fn default_state_dir() -> PathBuf {
    let program_data = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    crate::registration::state_dir(&program_data)
}

/// Read and parse a startup profile. With `require_admin_owned` (the installed service), a
/// file a non-administrator could have changed is rejected instead of applied.
pub fn load_startup_profile(path: &Path, require_admin_owned: bool) -> StartupProfile {
    if require_admin_owned {
        match crate::trust::untrusted_reason(path) {
            Ok(None) => {}
            Ok(Some(why)) => return StartupProfile::Rejected(why),
            Err(e) => {
                return StartupProfile::Rejected(format!(
                    "can't check who can change {}: {e}",
                    path.display()
                ));
            }
        }
    }
    match std::fs::read_to_string(path) {
        Err(e) => StartupProfile::Rejected(format!("can't read {}: {e}", path.display())),
        Ok(text) => match Profile::from_toml(&text) {
            Ok(p) => StartupProfile::Trusted(p),
            Err(e) => StartupProfile::Rejected(format!("{}: {e}", path.display())),
        },
    }
}

/// Serve until `stop` completes (or the pipe fails), then stop gracefully.
pub async fn run(cfg: HostConfig, stop: impl Future<Output = ()>) -> Result<()> {
    let Simulation { simulate, sim_load } = cfg.sim;
    let tick_ms = cfg.config.tick_ms;
    let opts =
        ServiceOptions::from_config(cfg.state_dir, &cfg.config, cfg.startup_profile, cfg.config_rejected);
    let mut service =
        Service::new(move || backends::open_all(simulate, sim_load), opts, CompatMatrix::builtin());
    if simulate.is_none() {
        // Simulated GPUs can't conflict with real tools.
        service = service.with_conflict_detector(conflicts::detect);
    }
    let svc = Arc::new(Mutex::new(service));

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
        let pipe = cfg.pipe.clone();
        tokio::spawn(async move { sol_ipc::pipe::serve(&pipe, handler, stop_rx).await })
    };
    tracing::info!(pipe = %cfg.pipe, "listening");

    let result = tokio::select! {
        () = stop => Ok(()),
        r = server => r.context("server task").and_then(|r| r.with_context(|| format!("serving {}", cfg.pipe))),
    };
    tracing::info!("stopping");
    let _ = stop_tx.send(true);
    let _ = control.await;
    svc.lock().unwrap_or_else(|e| e.into_inner()).shutdown();
    result
}
