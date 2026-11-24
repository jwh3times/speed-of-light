//! Opening every vendor backend and reporting what was found. Shared by the service and the
//! CLI's direct (no-service) commands.

use serde::Serialize;
use sol_core::backend::{Backend, BackendError, GpuDevice};
use sol_core::sim::{SimBackend, SimConfig, SimulatedGpu};

#[derive(Debug, Clone, Serialize)]
pub struct BackendStatus {
    pub backend: &'static str,
    /// `ok`, `not_present`, `unavailable` or `error`.
    pub status: &'static str,
    pub detail: String,
    pub devices: usize,
}

#[derive(Debug, Default)]
pub struct Opened {
    pub devices: Vec<Box<dyn GpuDevice>>,
    pub statuses: Vec<BackendStatus>,
}

fn record(out: &mut Opened, name: &'static str, result: Result<Box<dyn Backend>, BackendError>) {
    let (status, detail, devices) = match result.and_then(|mut b| b.devices()) {
        Ok(devs) => {
            let n = devs.len();
            out.devices.extend(devs);
            ("ok", format!("{n} device(s)"), n)
        }
        Err(BackendError::NotPresent(d)) => ("not_present", d, 0),
        Err(BackendError::Unavailable(d)) => ("unavailable", d, 0),
        Err(BackendError::Init(d)) => ("error", d, 0),
    };
    out.statuses.push(BackendStatus { backend: name, status, detail, devices });
}

/// Open real hardware backends, or `simulate` simulated GPUs instead.
pub fn open_all(simulate: Option<u32>, sim_load: f32) -> Opened {
    let mut out = Opened::default();
    if let Some(n) = simulate {
        let gpus = (0..n.max(1))
            .map(|i| {
                let cfg = SimConfig { name: format!("Simulated GPU {i}"), ..SimConfig::default() };
                let gpu = SimulatedGpu::with_id(cfg, &format!("sim:{i}")).real_time();
                gpu.set_load(sim_load);
                gpu
            })
            .collect();
        record(&mut out, "simulated", Ok(Box::new(SimBackend::new(gpus))));
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
