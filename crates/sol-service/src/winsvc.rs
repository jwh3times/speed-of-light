//! The Windows service itself: what the Service Control Manager starts, and `install` /
//! `uninstall`. Kept thin: the registration is built by [`crate::registration`], the files
//! are trusted by [`crate::trust`], and the service runs [`crate::host::run`].

use crate::cli::Settings;
use crate::host::{self, HostConfig};
use crate::registration::{self, LOG_DIR, Registration, SERVICE_NAME, STARTUP_PROFILE_FILE};
use crate::{backends, reset_to_stock, trust};
use anyhow::{Context, Result, bail};
use sol_core::profile::Profile;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo,
    ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
/// How long the Service Control Manager should allow a graceful stop to take.
const STOP_WAIT_HINT: Duration = Duration::from_secs(15);

/// Whether the service is installed and running, as a client sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledState {
    NotInstalled,
    Stopped,
    Starting,
    Running,
    Stopping,
    /// The Service Control Manager couldn't be asked.
    Unknown(String),
}

impl InstalledState {
    /// What to tell someone whose client can't reach the service.
    pub fn explain(&self) -> String {
        match self {
            InstalledState::NotInstalled => {
                "the service is not installed: run `sol-service install` from an elevated terminal \
                 (or `sol-service run` for a console session)"
                    .into()
            }
            InstalledState::Stopped => format!(
                "the service is installed but stopped: start it with `sc start {SERVICE_NAME}` from an \
                 elevated terminal, or from the Services console"
            ),
            InstalledState::Starting => "the service is still starting; try again in a few seconds".into(),
            InstalledState::Stopping => "the service is stopping".into(),
            InstalledState::Running => "the service is running but isn't answering on its pipe".into(),
            InstalledState::Unknown(e) => format!("is the service running? (couldn't ask Windows: {e})"),
        }
    }
}

/// Whether a console `sol-service run` may open the real GPUs. Not while the installed service
/// owns them: the console's startup takeover would pull the fans away from the service's curve
/// before the console failed on the taken pipe name.
pub fn console_may_open_real_gpus(installed: &InstalledState) -> Result<(), String> {
    let state = match installed {
        InstalledState::Running => "running",
        InstalledState::Starting => "starting",
        _ => return Ok(()),
    };
    Err(format!(
        "the installed service is {state}; it already owns the GPUs. Stop it first, or use `--simulate N`"
    ))
}

fn not_installed(e: &windows_service::Error) -> bool {
    matches!(e, windows_service::Error::Winapi(io) if io.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST))
}

/// Read-only; needs no elevation.
pub fn installed_state() -> InstalledState {
    let manager = match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) {
        Ok(m) => m,
        Err(e) => return InstalledState::Unknown(e.to_string()),
    };
    let service = match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Ok(s) => s,
        Err(e) if not_installed(&e) => return InstalledState::NotInstalled,
        Err(e) => return InstalledState::Unknown(e.to_string()),
    };
    match service.query_status() {
        Ok(s) => match s.current_state {
            ServiceState::Stopped => InstalledState::Stopped,
            ServiceState::StartPending => InstalledState::Starting,
            ServiceState::StopPending => InstalledState::Stopping,
            _ => InstalledState::Running,
        },
        Err(e) => InstalledState::Unknown(e.to_string()),
    }
}

fn require_elevation(what: &str) -> Result<()> {
    if !is_elevated() {
        bail!(
            "`sol-service {what}` needs an elevated terminal: right-click your terminal, choose \"Run as administrator\", and run it again"
        );
    }
    Ok(())
}

fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token: HANDLE = std::ptr::null_mut();
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut len = 0u32;
    // SAFETY: GetCurrentProcess returns a pseudo-handle; `token` is a valid out-pointer and is
    // closed below; `elevation` is a TOKEN_ELEVATION whose size we pass.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

fn service_info(reg: &Registration) -> ServiceInfo {
    ServiceInfo {
        name: reg.name.into(),
        display_name: reg.display_name.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: match reg.start {
            registration::StartType::Automatic => ServiceStartType::AutoStart,
        },
        error_control: ServiceErrorControl::Normal,
        executable_path: reg.executable.clone(),
        launch_arguments: reg.arguments.iter().map(OsString::from).collect(),
        dependencies: Vec::new(),
        // `None` is LocalSystem.
        account_name: match reg.account {
            registration::Account::LocalSystem => None,
        },
        account_password: None,
    }
}

fn wait_for(
    service: &windows_service::service::Service,
    state: ServiceState,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    loop {
        if service.query_status()?.current_state == state {
            return Ok(());
        }
        if start.elapsed() > timeout {
            bail!("the service didn't reach {state:?} within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Ask the service to stop and wait for its graceful stop to finish.
fn stop_and_wait(service: &windows_service::service::Service) -> Result<()> {
    if service.query_status()?.current_state != ServiceState::Stopped {
        println!("stopping the running service (fans return to firmware control)...");
        if let Err(e) = service.stop() {
            tracing::debug!(error = %e, "stop request failed; it may already be stopping");
        }
        wait_for(service, ServiceState::Stopped, Duration::from_secs(30))?;
    }
    Ok(())
}

/// Copy, retrying briefly: a just-stopped service's executable can stay locked for a moment.
fn copy_with_retry(from: &Path, to: &Path) -> Result<()> {
    let mut last = None;
    for _ in 0..20 {
        match std::fs::copy(from, to) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(last.map_or_else(|| anyhow::anyhow!("copy failed"), anyhow::Error::from))
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))
}

/// Install, or upgrade in place, and start. See the spec in #16 and ADR 0002.
pub fn install(settings: &Settings, startup_profile: Option<&Path>) -> Result<()> {
    require_elevation("install")?;
    let profile_text = match startup_profile {
        None => None,
        Some(p) => {
            let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            Profile::from_toml(&text).with_context(|| format!("checking {}", p.display()))?;
            Some(text)
        }
    };

    let reg = registration::registration(&program_files(), settings);
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP;
    let existing = match manager.open_service(SERVICE_NAME, access) {
        Ok(s) => Some(s),
        Err(e) if not_installed(&e) => None,
        Err(e) => return Err(e).context("opening the existing service"),
    };
    if let Some(s) = &existing {
        stop_and_wait(s)?;
    }

    // The binary: an administrator-only copy under Program Files, never where we ran from.
    let install_dir = reg.executable.parent().map(Path::to_path_buf).unwrap_or_else(program_files);
    std::fs::create_dir_all(&install_dir).with_context(|| format!("creating {}", install_dir.display()))?;
    let me = std::env::current_exe().context("finding this executable")?;
    let same = std::fs::canonicalize(&me).ok() == std::fs::canonicalize(&reg.executable).ok();
    if !same {
        copy_with_retry(&me, &reg.executable)?;
    }

    // The state folder: take it back first (a standard user may have created it), write the
    // startup profile, then reset again so the new file is administrator-owned too.
    let state = host::default_state_dir();
    std::fs::create_dir_all(&state).with_context(|| format!("creating {}", state.display()))?;
    trust::harden_dir(&state, true).with_context(|| format!("locking down {}", state.display()))?;
    let stored = state.join(STARTUP_PROFILE_FILE);
    match &profile_text {
        Some(text) => {
            std::fs::write(&stored, text).with_context(|| format!("writing {}", stored.display()))?
        }
        None => match std::fs::remove_file(&stored) {
            Ok(()) => println!("removed the previous startup profile"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", stored.display())),
        },
    }
    trust::harden_dir(&state, true).with_context(|| format!("locking down {}", state.display()))?;

    let info = service_info(&reg);
    let service = match existing {
        Some(s) => {
            s.change_config(&info).context("updating the service")?;
            s
        }
        None => manager.create_service(&info, access).context("creating the service")?,
    };
    service.set_description(reg.description)?;
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(reg.recovery.reset_after),
        reboot_msg: None,
        command: None,
        actions: Some(
            reg.recovery
                .restart_after
                .iter()
                .map(|&delay| ServiceAction { action_type: ServiceActionType::Restart, delay })
                .collect(),
        ),
    })?;
    service.set_failure_actions_on_non_crash_failures(reg.recovery.on_error_exit)?;
    service.start::<&str>(&[]).context("starting the service")?;
    wait_for(&service, ServiceState::Running, Duration::from_secs(30))?;

    println!("installed and started {} ({})", reg.display_name, reg.name);
    println!("  binary:          {}", reg.executable.display());
    println!("  state and logs:  {}", state.display());
    match startup_profile {
        Some(p) => println!(
            "  startup profile: copied from {} (edit it and run install again to change it)",
            p.display()
        ),
        None => println!("  startup profile: none"),
    }
    if settings.simulate.is_some() {
        println!("  simulated GPUs only: no real GPU is touched");
    }
    Ok(())
}

/// Remove `path` with `remove`, saying so; already gone is fine, and a failure is only a warning.
fn report_removal(path: &Path, remove: fn(&Path) -> std::io::Result<()>) {
    match remove(path) {
        Ok(()) => println!("removed {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("warning: couldn't remove {}: {e}", path.display()),
    }
}

/// Stop, reset every GPU to stock, and remove. Returns the process exit code: non-zero when a
/// GPU isn't verifiably back at stock.
pub fn uninstall(purge: bool) -> Result<i32> {
    require_elevation("uninstall")?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::QUERY_CONFIG
        | ServiceAccess::STOP
        | ServiceAccess::DELETE;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(s) => Some(s),
        Err(e) if not_installed(&e) => {
            println!("the service isn't installed; cleaning up anyway");
            None
        }
        Err(e) => return Err(e).context("opening the service"),
    };
    // An install with --simulate never touched a real GPU, so its uninstall mustn't either.
    let stored = service.as_ref().and_then(|s| s.query_config().ok()).map(|c| c.executable_path);
    let resets_real =
        registration::uninstall_resets_real_gpus(stored.as_deref().map(|p| p.to_string_lossy()).as_deref());
    if let Some(s) = &service {
        stop_and_wait(s)?;
    }

    let mut code = 0;
    if !resets_real {
        println!("installed with --simulate: no real GPU to reset");
    } else {
        let opened = backends::open_all(None, 0.0);
        let result = reset_to_stock(opened.devices);
        for r in &result.reports {
            println!("{}: {}", r.device, if r.all_applied() { "back at stock" } else { "NOT back at stock" });
        }
        for f in &result.failures {
            eprintln!("error: {f}");
        }
        if !result.ok() {
            eprintln!("some settings may still be applied until the next reboot or driver reset");
            code = 1;
        }
    }

    if let Some(s) = &service {
        s.delete().context("removing the service")?;
        println!("removed the service");
    }
    report_removal(&registration::install_dir(&program_files()), |p| std::fs::remove_dir_all(p));
    let state = host::default_state_dir();
    if purge {
        report_removal(&state, |p| std::fs::remove_dir_all(p));
    } else {
        report_removal(&state.join(STARTUP_PROFILE_FILE), |p| std::fs::remove_file(p));
        println!("kept the logs and the crash guard in {} (`--purge` removes them)", state.display());
    }
    Ok(code)
}

static SERVICE_SETTINGS: OnceLock<Settings> = OnceLock::new();

windows_service::define_windows_service!(ffi_service_main, service_main);

/// Hand this thread to the Service Control Manager. Returns once the service has stopped.
pub fn run_dispatcher(settings: Settings) -> Result<()> {
    let _ = SERVICE_SETTINGS.set(settings);
    windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("connecting to the Service Control Manager (this command is only for the service itself)")
}

fn service_main(_arguments: Vec<OsString>) {
    let Some(settings) = SERVICE_SETTINGS.get().cloned() else { return };
    let state_dir = host::default_state_dir();
    let _log = init_file_logging(&state_dir.join(LOG_DIR), settings.log_level);
    if let Err(e) = run_service(settings, state_dir) {
        tracing::error!("service failed: {e:#}");
    }
}

fn init_file_logging(
    dir: &Path,
    level: tracing::Level,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("sol-service")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt().with_writer(writer).with_ansi(false).with_max_level(level).try_init().ok()?;
    Some(guard)
}

fn run_service(settings: Settings, state_dir: PathBuf) -> Result<()> {
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let status = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let report_status =
        |state: ServiceState, accept: ServiceControlAccept, exit: ServiceExitCode, wait: Duration| {
            status.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: accept,
                exit_code: exit,
                checkpoint: 0,
                wait_hint: wait,
                process_id: None,
            })
        };
    report_status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceExitCode::Win32(0),
        Duration::ZERO,
    )?;
    tracing::info!(version = crate::VERSION, ?settings, "service started");

    let stored = state_dir.join(STARTUP_PROFILE_FILE);
    let startup_profile = stored.exists().then(|| host::load_startup_profile(&stored, true));
    let cfg = HostConfig { settings, state_dir, pipe: sol_ipc::PIPE_NAME.into(), startup_profile };
    let stop = async {
        let _ = stop_rx.changed().await;
        let _ = report_status(
            ServiceState::StopPending,
            ServiceControlAccept::empty(),
            ServiceExitCode::Win32(0),
            STOP_WAIT_HINT,
        );
    };
    let result = tokio::runtime::Runtime::new()
        .context("starting the async runtime")
        .and_then(|rt| rt.block_on(host::run(cfg, stop)));

    // An error exit counts as a failure, so the recovery actions restart the service.
    let exit = match &result {
        Ok(()) => ServiceExitCode::Win32(0),
        Err(_) => ServiceExitCode::ServiceSpecific(1),
    };
    report_status(ServiceState::Stopped, ServiceControlAccept::empty(), exit, Duration::ZERO)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_console_run_refuses_real_gpus_while_the_installed_service_owns_them() {
        assert!(console_may_open_real_gpus(&InstalledState::Running).is_err());
        assert!(console_may_open_real_gpus(&InstalledState::Starting).is_err());
        assert!(console_may_open_real_gpus(&InstalledState::NotInstalled).is_ok());
        assert!(console_may_open_real_gpus(&InstalledState::Stopped).is_ok());
    }
}
