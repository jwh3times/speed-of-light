//! Crash guard: never re-apply a profile that took the machine down.
//!
//! Before applying a profile the service *arms* the guard by writing a sentinel file. Once the
//! profile has run for the stability window without the process dying, the service *disarms*
//! it. A graceful shutdown also disarms it. If the sentinel is still there at the next start,
//! the last apply didn't survive (a BSOD, a hard lock, a power-off during a stress test), so
//! the service skips its startup profile and leaves the card at stock until the user clears
//! the guard.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sentinel {
    pub profile: String,
    pub armed_at_unix: u64,
    pub pid: u32,
}

#[derive(Debug, Clone)]
pub struct CrashGuard {
    path: PathBuf,
}

impl CrashGuard {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        CrashGuard { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `Some` means the previous apply did not survive its stability window.
    pub fn check(&self) -> io::Result<Option<Sentinel>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(serde_json::from_str(&text).unwrap_or(Sentinel {
                // An unreadable sentinel still means "armed": a half-written file is exactly
                // what a crash mid-write leaves behind.
                profile: "<unreadable sentinel>".into(),
                armed_at_unix: 0,
                pid: 0,
            }))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn arm(&self, profile: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let sentinel = Sentinel {
            profile: profile.to_string(),
            armed_at_unix: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            pid: std::process::id(),
        };
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(&sentinel).map_err(io::Error::other)?)?;
        fs::rename(&tmp, &self.path)
    }

    pub fn disarm(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_check_disarm_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let guard = CrashGuard::new(dir.path().join("state").join("guard.json"));
        assert_eq!(guard.check().unwrap(), None);
        guard.arm("OC").unwrap();
        let s = guard.check().unwrap().unwrap();
        assert_eq!(s.profile, "OC");
        assert_eq!(s.pid, std::process::id());
        guard.disarm().unwrap();
        assert_eq!(guard.check().unwrap(), None);
        guard.disarm().unwrap(); // idempotent
    }

    #[test]
    fn corrupt_sentinel_still_counts_as_armed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guard.json");
        fs::write(&path, b"{\"profile\": \"half").unwrap();
        assert!(CrashGuard::new(path).check().unwrap().is_some());
    }
}
