//! The service config: the administrator-owned settings the installed service runs with
//! (`%ProgramData%\SpeedOfLight\config.toml`, copied there by `install --config`). See
//! ADR 0003 for what happens when the file can't be trusted.

use serde::Deserialize;
use sol_core::tuner::SafetyPolicy;
use std::fmt;
use std::ops::RangeInclusive;
use std::path::Path;

/// The service config file's schema. Bump it by the same rule as `profile::SCHEMA_VERSION`.
pub const SCHEMA_VERSION: u32 = 1;

/// The file as written. Every key but `schema` is optional; unknown keys are an error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    #[serde(default)]
    max_power_percent: Option<f32>,
    #[serde(default)]
    max_core_offset_mhz: Option<i32>,
    #[serde(default)]
    max_memory_offset_mhz: Option<i32>,
    #[serde(default)]
    allow_unverified: bool,
    tick_ms: Option<u64>,
    stability_secs: Option<u64>,
    log_level: Option<String>,
}

/// The control-loop period, in milliseconds. The `--tick-ms` flag takes the same range.
pub const TICK_MS: RangeInclusive<u64> = 250..=5000;
pub const DEFAULT_TICK_MS: u64 = 1000;
/// The stability window, in seconds. Shorter would let a profile that hangs the PC a little
/// later count as having survived. The `--stability-secs` flag takes the same range.
pub const STABILITY_SECS: RangeInclusive<u64> = 30..=3600;
pub const DEFAULT_STABILITY_SECS: u64 = 120;
/// The names `log_level` accepts. Tracing's own parser also takes numbers; the file doesn't.
pub const LOG_LEVELS: [(&str, tracing::Level); 5] = [
    ("error", tracing::Level::ERROR),
    ("warn", tracing::Level::WARN),
    ("info", tracing::Level::INFO),
    ("debug", tracing::Level::DEBUG),
    ("trace", tracing::Level::TRACE),
];
pub const DEFAULT_LOG_LEVEL: tracing::Level = tracing::Level::INFO;

/// The service config, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceConfig {
    /// The policy caps and `allow_unverified`.
    pub policy: SafetyPolicy,
    pub tick_ms: u64,
    pub stability_secs: u64,
    pub log_level: tracing::Level,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        ServiceConfig {
            policy: SafetyPolicy::default(),
            tick_ms: DEFAULT_TICK_MS,
            stability_secs: DEFAULT_STABILITY_SECS,
            log_level: DEFAULT_LOG_LEVEL,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigError {
    /// Not TOML, a missing `schema`, an unknown key or a value of the wrong type.
    Parse(String),
    /// Well-formed, but these values are not allowed.
    Invalid(Vec<String>),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Parse(e) => write!(f, "service config is not valid: {e}"),
            ConfigError::Invalid(issues) => {
                write!(f, "service config is invalid:\n  - {}", issues.join("\n  - "))
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// A log level by the name the file and `--log-level` spell it.
pub fn log_level_named(name: &str) -> Option<tracing::Level> {
    LOG_LEVELS.iter().find(|(n, _)| *n == name).map(|&(_, level)| level)
}

impl ServiceConfig {
    pub fn from_toml(text: &str) -> Result<ServiceConfig, ConfigError> {
        let file: File = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let mut issues = Vec::new();
        if file.schema != SCHEMA_VERSION {
            issues.push(format!(
                "schema {} is not supported (this build reads schema {SCHEMA_VERSION})",
                file.schema
            ));
        }
        if let Some(p) = file.max_power_percent
            && !(p.is_finite() && p > 0.0)
        {
            issues.push(format!("max_power_percent = {p} must be above 0"));
        }
        for (key, cap) in [
            ("max_core_offset_mhz", file.max_core_offset_mhz),
            ("max_memory_offset_mhz", file.max_memory_offset_mhz),
        ] {
            if let Some(mhz) = cap
                && mhz < 0
            {
                issues.push(format!("{key} = {mhz} must be 0 or more (it caps the offset either way)"));
            }
        }
        let tick_ms = in_range(&mut issues, "tick_ms", file.tick_ms, DEFAULT_TICK_MS, TICK_MS);
        let stability_secs = in_range(
            &mut issues,
            "stability_secs",
            file.stability_secs,
            DEFAULT_STABILITY_SECS,
            STABILITY_SECS,
        );
        let log_level = match file.log_level.as_deref() {
            None => DEFAULT_LOG_LEVEL,
            Some(name) => log_level_named(name).unwrap_or_else(|| {
                let names: Vec<_> = LOG_LEVELS.iter().map(|(n, _)| *n).collect();
                issues.push(format!("log_level = {name:?} must be one of {}", names.join(", ")));
                DEFAULT_LOG_LEVEL
            }),
        };
        if !issues.is_empty() {
            return Err(ConfigError::Invalid(issues));
        }
        Ok(ServiceConfig {
            policy: SafetyPolicy {
                allow_unverified: file.allow_unverified,
                max_power_percent: file.max_power_percent,
                max_core_offset_mhz: file.max_core_offset_mhz,
                max_memory_offset_mhz: file.max_memory_offset_mhz,
            },
            tick_ms,
            stability_secs,
            log_level,
        })
    }
}

/// `value`, or `default` when the key is absent; a value outside `range` is an issue.
fn in_range(
    issues: &mut Vec<String>,
    key: &str,
    value: Option<u64>,
    default: u64,
    range: RangeInclusive<u64>,
) -> u64 {
    let v = value.unwrap_or(default);
    if !range.contains(&v) {
        issues.push(format!("{key} = {v} must be in {}..={}", range.start(), range.end()));
    }
    v
}

/// The installed service's config as the service read it at startup.
#[derive(Debug, Clone, PartialEq)]
pub enum Loaded {
    /// No file: the defaults, with no policy caps.
    Missing,
    Trusted(ServiceConfig),
    /// Present but untrusted, unreadable or invalid, for this reason. The service then blocks
    /// every write except a return to stock (ADR 0003).
    Rejected(String),
}

/// Read and check a service config someone named on a command line: what `install --config`
/// copies, and what a console `run --config` uses. Nobody's ownership is checked: the person
/// running the command chose the file. Returns its text too, so `install` copies exactly what
/// it checked.
pub fn read_named(path: &Path) -> Result<(String, ServiceConfig), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let cfg = ServiceConfig::from_toml(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((text, cfg))
}

/// Load the installed service's copy. `untrusted` says why a non-administrator could have
/// changed the file (`None`: only administrators can); such a file is never parsed.
pub fn load_installed(
    path: &Path,
    untrusted: impl FnOnce(&Path) -> Result<Option<String>, String>,
) -> Loaded {
    if !path.exists() {
        return Loaded::Missing;
    }
    match untrusted(path) {
        Ok(None) => {}
        Ok(Some(why)) => return Loaded::Rejected(why),
        Err(e) => return Loaded::Rejected(format!("can't check who can change {}: {e}", path.display())),
    }
    match read_named(path) {
        Ok((_, cfg)) => Loaded::Trusted(cfg),
        Err(e) => Loaded::Rejected(e),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_config_setting_every_key_gives_that_policy_and_those_settings() {
        let cfg = ServiceConfig::from_toml(
            "schema = 1\n\
             max_power_percent = 90\n\
             max_core_offset_mhz = 150\n\
             max_memory_offset_mhz = 500\n\
             allow_unverified = true\n\
             tick_ms = 500\n\
             stability_secs = 300\n\
             log_level = \"debug\"\n",
        )
        .unwrap();
        assert_eq!(
            cfg,
            ServiceConfig {
                policy: SafetyPolicy {
                    allow_unverified: true,
                    max_power_percent: Some(90.0),
                    max_core_offset_mhz: Some(150),
                    max_memory_offset_mhz: Some(500),
                },
                tick_ms: 500,
                stability_secs: 300,
                log_level: tracing::Level::DEBUG,
            }
        );
    }

    #[test]
    fn a_config_with_only_its_schema_gives_the_defaults() {
        let cfg = ServiceConfig::from_toml("schema = 1\n").unwrap();
        assert_eq!(cfg, ServiceConfig::default());
        assert_eq!(cfg.policy, SafetyPolicy::default());
        assert_eq!((cfg.tick_ms, cfg.stability_secs, cfg.log_level), (1000, 120, tracing::Level::INFO));
    }

    #[test]
    fn a_config_that_is_wrong_is_rejected_naming_what_is_wrong() {
        for (body, names) in [
            ("", "schema"),
            ("schema = 2", "schema"),
            ("schema = 1\nmax_power_percnt = 90", "max_power_percnt"),
            ("schema = 1\ntick_ms = 249", "tick_ms"),
            ("schema = 1\ntick_ms = 5001", "tick_ms"),
            ("schema = 1\nstability_secs = 29", "stability_secs"),
            ("schema = 1\nstability_secs = 3601", "stability_secs"),
            ("schema = 1\nmax_power_percent = 0", "max_power_percent"),
            ("schema = 1\nmax_power_percent = -5", "max_power_percent"),
            ("schema = 1\nmax_power_percent = nan", "max_power_percent"),
            ("schema = 1\nmax_power_percent = inf", "max_power_percent"),
            ("schema = 1\nmax_core_offset_mhz = -1", "max_core_offset_mhz"),
            ("schema = 1\nmax_memory_offset_mhz = -100", "max_memory_offset_mhz"),
            ("schema = 1\nlog_level = \"loud\"", "log_level"),
            // Tracing would read "3" as a level; the file takes only the names.
            ("schema = 1\nlog_level = \"3\"", "log_level"),
        ] {
            let err = ServiceConfig::from_toml(body).expect_err(body);
            assert!(err.to_string().contains(names), "{body:?} -> {err}");
        }
    }

    fn trusted(_: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn write(dir: &tempfile::TempDir, text: &str) -> std::path::PathBuf {
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_missing_service_config_means_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_installed(&dir.path().join("config.toml"), trusted), Loaded::Missing);
    }

    #[test]
    fn a_trusted_valid_service_config_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "schema = 1\ntick_ms = 500\n");
        let Loaded::Trusted(cfg) = load_installed(&path, trusted) else { panic!("not trusted") };
        assert_eq!(cfg.tick_ms, 500);
    }

    #[test]
    fn an_invalid_service_config_is_rejected_naming_the_file_and_the_problem() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "schema = 1\ntick_ms = 1\n");
        let Loaded::Rejected(why) = load_installed(&path, trusted) else { panic!("not rejected") };
        assert!(why.contains("config.toml") && why.contains("tick_ms"));
    }

    #[test]
    fn a_service_config_others_could_change_is_rejected_even_when_it_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(&dir, "schema = 1\n");
        let users_can_write = |_: &Path| Ok(Some("BUILTIN\\Users can write it".to_string()));
        let Loaded::Rejected(why) = load_installed(&path, users_can_write) else { panic!("not rejected") };
        assert!(why.contains("BUILTIN\\Users can write it"));
        let unknown = |_: &Path| Err("access denied".to_string());
        let Loaded::Rejected(why) = load_installed(&path, unknown) else { panic!("not rejected") };
        assert!(why.contains("access denied"));
    }

    #[test]
    fn install_refuses_a_service_config_that_is_not_valid() {
        let dir = tempfile::tempdir().unwrap();
        let good = write(&dir, "schema = 1\n");
        assert_eq!(read_named(&good).unwrap(), ("schema = 1\n".to_string(), ServiceConfig::default()));
        let bad = write(&dir, "schema = 1\nno_such_key = 1\n");
        let err = read_named(&bad).unwrap_err();
        assert!(err.contains("config.toml") && err.contains("no_such_key"), "{err}");
        assert!(read_named(&dir.path().join("absent.toml")).is_err());
    }

    #[test]
    fn the_example_service_config_parses_and_changes_nothing_until_edited() {
        let example = include_str!("../../../examples/config.toml");
        assert_eq!(ServiceConfig::from_toml(example).unwrap(), ServiceConfig::default());
        // Every key is shown, commented out, so uncommenting one is the whole edit.
        for key in [
            "max_power_percent",
            "max_core_offset_mhz",
            "max_memory_offset_mhz",
            "allow_unverified",
            "tick_ms",
            "stability_secs",
            "log_level",
        ] {
            assert!(example.lines().any(|l| l.starts_with(&format!("# {key} = "))), "{key} is missing");
        }
    }

    #[test]
    fn the_range_limits_themselves_are_accepted() {
        for body in [
            "schema = 1\ntick_ms = 250\nstability_secs = 30",
            "schema = 1\ntick_ms = 5000\nstability_secs = 3600",
            "schema = 1\nmax_core_offset_mhz = 0\nmax_memory_offset_mhz = 0",
            "schema = 1\nlog_level = \"error\"",
            "schema = 1\nlog_level = \"trace\"",
        ] {
            ServiceConfig::from_toml(body).unwrap_or_else(|e| panic!("{body:?} -> {e}"));
        }
    }
}
