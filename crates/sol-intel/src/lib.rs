//! Intel backend. **Detection only.**
//!
//! The plan is the Intel Graphics Control Library (IGCL, `ControlLib.dll`, shipped with Arc
//! drivers). Its overclocking API (`ctlOverclock*`) covers frequency offset, voltage offset,
//! power and temperature limits, and fan control (`ctlFan*`). See `docs/vendor-apis.md`.
//!
//! Until someone with Arc hardware implements and verifies it, this backend reports the
//! runtime as found-but-unsupported rather than pretending to work.

use sol_core::backend::{Backend, BackendError};
use std::path::PathBuf;

pub const IGCL_DLL: &str = "ControlLib.dll";

fn system32() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32")
}

/// Finds the IGCL runtime by file presence. It doesn't load the DLL: probing must not run
/// vendor code we can't use yet.
pub fn open() -> Result<Box<dyn Backend>, BackendError> {
    let path = system32().join(IGCL_DLL);
    if path.is_file() {
        Err(BackendError::Unavailable(format!(
            "Intel IGCL runtime found at {}, but the Intel backend is not implemented yet",
            path.display()
        )))
    } else {
        Err(BackendError::NotPresent("no Intel IGCL runtime (no Intel Arc driver installed)".into()))
    }
}
