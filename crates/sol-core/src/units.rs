//! Unit newtypes. Every hardware value crosses an API boundary with its unit in the type, so a
//! milliwatt can't be passed where a watt is expected and a duty cycle can't leave 0..=100.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Temperature in degrees Celsius.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Celsius(pub f32);

impl fmt::Display for Celsius {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.0} °C", self.0)
    }
}

/// A percentage in `0.0..=100.0`. Construction is checked, so a `Percent` is always valid.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Percent(f32);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0} is not a percentage in 0..=100")]
pub struct PercentRangeError(pub f32);

impl Percent {
    pub const ZERO: Percent = Percent(0.0);
    pub const FULL: Percent = Percent(100.0);

    pub fn new(value: f32) -> Result<Self, PercentRangeError> {
        if value.is_finite() && (0.0..=100.0).contains(&value) {
            Ok(Percent(value))
        } else {
            Err(PercentRangeError(value))
        }
    }

    /// Clamps into range. A non-finite input becomes 100 %, because every caller that clamps
    /// is computing a fan duty, and an unknown duty must fail towards cooling.
    pub fn clamped(value: f32) -> Self {
        if value.is_finite() { Percent(value.clamp(0.0, 100.0)) } else { Percent::FULL }
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Percent {
    type Error = PercentRangeError;
    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Percent::new(value)
    }
}

impl From<Percent> for f32 {
    fn from(p: Percent) -> f32 {
        p.0
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.0} %", self.0)
    }
}

/// Power in milliwatts (NVML's native unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Milliwatts(pub u32);

impl Milliwatts {
    pub fn from_watts(watts: f32) -> Self {
        Milliwatts((watts.max(0.0) * 1000.0).round() as u32)
    }

    pub fn watts(self) -> f32 {
        self.0 as f32 / 1000.0
    }
}

impl fmt::Display for Milliwatts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1} W", self.watts())
    }
}

/// An absolute clock frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Mhz(pub u32);

impl fmt::Display for Mhz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} MHz", self.0)
    }
}

/// A signed clock offset relative to the card's stock V/F curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MhzOffset(pub i32);

impl fmt::Display for MhzOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:+} MHz", self.0)
    }
}

/// Fan speed in revolutions per minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rpm(pub u32);

impl fmt::Display for Rpm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} RPM", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_rejects_out_of_range_and_nan() {
        assert!(Percent::new(-0.1).is_err());
        assert!(Percent::new(100.1).is_err());
        assert!(Percent::new(f32::NAN).is_err());
        assert!(Percent::new(0.0).is_ok());
        assert!(Percent::new(100.0).is_ok());
    }

    #[test]
    fn percent_clamped_fails_towards_cooling() {
        assert_eq!(Percent::clamped(f32::NAN), Percent::FULL);
        assert_eq!(Percent::clamped(150.0), Percent::FULL);
        assert_eq!(Percent::clamped(-5.0), Percent::ZERO);
    }

    #[test]
    fn percent_deserialize_is_checked() {
        assert!(serde_json::from_str::<Percent>("101").is_err());
        assert_eq!(serde_json::from_str::<Percent>("42").ok(), Percent::new(42.0).ok());
    }

    #[test]
    fn watts_round_trip() {
        assert_eq!(Milliwatts::from_watts(320.0), Milliwatts(320_000));
        assert_eq!(Milliwatts(287_500).watts(), 287.5);
        assert_eq!(Milliwatts::from_watts(-3.0), Milliwatts(0));
    }
}
