//! AMD backend. **Detection only.**
//!
//! The plan is AMD's public ADLX SDK (`amdadlx64.dll`, shipped with Adrenalin), which exposes
//! manual GPU tuning (clocks, voltage, power, fan curves) through `IADLXGPUTuningServices`.
//! ADLX is a C++/COM-style interface with a C binding. See `docs/vendor-apis.md`.
//!
//! Until someone with RDNA hardware implements and verifies it, this backend reports the
//! runtime as found-but-unsupported rather than pretending to work.

use sol_core::backend::{Backend, BackendError};
use std::path::PathBuf;

pub const ADLX_DLL: &str = "amdadlx64.dll";

fn system32() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32")
}

/// Finds the ADLX runtime by file presence. It doesn't load the DLL: probing must not run
/// vendor code we can't use yet.
pub fn open() -> Result<Box<dyn Backend>, BackendError> {
    let path = system32().join(ADLX_DLL);
    if path.is_file() {
        Err(BackendError::Unavailable(format!(
            "AMD ADLX runtime found at {}, but the AMD backend is not implemented yet",
            path.display()
        )))
    } else {
        Err(BackendError::NotPresent("no AMD ADLX runtime (no AMD GPU driver installed)".into()))
    }
}
