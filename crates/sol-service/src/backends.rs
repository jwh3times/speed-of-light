//! Opening every vendor backend and reporting what was found. Shared by the service and the
//! CLI's direct (no-service) commands.

use sol_core::backend::{Backend, BackendError, GpuDevice};
use sol_core::sim::{SimBackend, SimConfig, SimulatedGpu};
pub use sol_ipc::{BackendState, BackendStatus};

#[derive(Debug, Default)]
pub struct Opened {
    pub devices: Vec<Box<dyn GpuDevice>>,
    pub statuses: Vec<BackendStatus>,
}

impl Opened {
    /// Trying again may find a GPU: some backend's runtime is installed but not ready yet, or
    /// failed unexpectedly. Only a missing or unusable runtime is given up on.
    pub fn worth_retrying(&self) -> bool {
        self.statuses.iter().any(|s| matches!(s.status, BackendState::NotReady | BackendState::Error))
    }
}

fn record(out: &mut Opened, name: &'static str, result: Result<Box<dyn Backend>, BackendError>) {
    let (status, detail, devices) = match result.and_then(|mut b| b.devices()) {
        // A runtime that initialised but sees no GPU is treated as not ready: at boot the
        // driver can answer before it has enumerated the cards.
        Ok(devs) if devs.is_empty() => (BackendState::NotReady, "no devices yet".to_string(), 0),
        Ok(devs) => {
            let n = devs.len();
            out.devices.extend(devs);
            (BackendState::Ok, format!("{n} device(s)"), n)
        }
        Err(BackendError::NotPresent(d)) => (BackendState::NotPresent, d, 0),
        Err(BackendError::NotReady(d)) => (BackendState::NotReady, d, 0),
        Err(BackendError::Unavailable(d)) => (BackendState::Unavailable, d, 0),
        Err(BackendError::Init(d)) => (BackendState::Error, d, 0),
    };
    out.statuses.push(BackendStatus { backend: name.to_string(), status, detail, devices });
}

/// `n` simulated GPUs at `load` (0..1), on wall-clock time.
pub fn simulated(n: u32, load: f32) -> Vec<SimulatedGpu> {
    (0..n.max(1))
        .map(|i| {
            let cfg = SimConfig { name: format!("Simulated GPU {i}"), ..SimConfig::default() };
            let gpu = SimulatedGpu::with_id(cfg, &format!("sim:{i}")).real_time();
            gpu.set_load(load);
            gpu
        })
        .collect()
}

/// Open real hardware backends, or `simulate` simulated GPUs instead.
pub fn open_all(simulate: Option<u32>, sim_load: f32) -> Opened {
    let mut out = Opened::default();
    if let Some(n) = simulate {
        record(&mut out, "simulated", Ok(Box::new(SimBackend::new(simulated(n, sim_load)))));
        return out;
    }
    record(
        &mut out,
        "nvidia-nvml",
        sol_nvidia::NvidiaBackend::open().map(|b| Box::new(b) as Box<dyn Backend>),
    );
    record(&mut out, "amd-adlx", sol_amd::open());
    record(&mut out, "intel-igcl", sol_intel::open());
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use sol_core::sim::SimulatedGpu;

    fn status_of(result: Result<Box<dyn Backend>, BackendError>) -> (BackendState, Opened) {
        let mut out = Opened::default();
        record(&mut out, "test", result);
        (out.statuses[0].status, out)
    }

    #[test]
    fn a_driver_that_is_still_loading_is_not_ready_and_worth_retrying() {
        let (state, out) = status_of(Err(BackendError::NotReady("driver not loaded".into())));
        assert_eq!(state, BackendState::NotReady);
        assert!(out.worth_retrying());
    }

    #[test]
    fn a_runtime_that_sees_no_gpu_yet_is_not_ready() {
        let (state, out) = status_of(Ok(Box::new(SimBackend::new(Vec::new()))));
        assert_eq!(state, BackendState::NotReady);
        assert!(out.worth_retrying());
    }

    #[test]
    fn a_missing_runtime_is_not_retried() {
        let (state, out) = status_of(Err(BackendError::NotPresent("no nvml.dll".into())));
        assert_eq!(state, BackendState::NotPresent);
        assert!(!out.worth_retrying());
    }

    #[test]
    fn found_devices_are_ok() {
        let gpu = SimulatedGpu::new(SimConfig::default());
        let (state, out) = status_of(Ok(Box::new(SimBackend::new(vec![gpu]))));
        assert_eq!(state, BackendState::Ok);
        assert_eq!(out.devices.len(), 1);
    }
}
