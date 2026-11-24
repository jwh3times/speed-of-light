//! Software fan curves.
//!
//! Safety properties (each one has a test or a property test below):
//!
//! 1. Output is always a valid duty in `0..=100`.
//! 2. With the default (unlimited) ramp-up, output is never below the curve's value at the
//!    current temperature. Hysteresis and ramp-down only ever keep the fan *faster*.
//! 3. At or above `critical_temp`, output is 100 % immediately.
//! 4. If the temperature sensor fails for `sensor_fail_ticks` ticks in a row, control goes
//!    back to the firmware ([`FanCommand::FirmwareAuto`]) instead of holding a stale duty.
//! 5. A curve whose duty decreases as temperature rises is rejected at construction.

use crate::units::{Celsius, Percent};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub temp: Celsius,
    pub duty: Percent,
}

impl CurvePoint {
    pub fn new(temp: f32, duty: f32) -> Result<Self, CurveError> {
        if !temp.is_finite() {
            return Err(CurveError::NonFinite);
        }
        let duty = Percent::new(duty).map_err(|e| CurveError::DutyOutOfRange(e.0))?;
        Ok(CurvePoint { temp: Celsius(temp), duty })
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CurveError {
    #[error("a fan curve needs at least 2 points")]
    TooFewPoints,
    #[error("point {index}: temperatures must strictly increase")]
    TempNotIncreasing { index: usize },
    #[error("point {index}: duty must not decrease as temperature rises")]
    DutyDecreasing { index: usize },
    #[error("temperatures must be finite numbers")]
    NonFinite,
    #[error("duty {0} is outside 0..=100")]
    DutyOutOfRange(f32),
}

/// A validated, monotonic temperature-to-duty curve, linearly interpolated.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FanCurve {
    points: Vec<CurvePoint>,
}

impl FanCurve {
    pub fn new(points: Vec<CurvePoint>) -> Result<Self, CurveError> {
        if points.len() < 2 {
            return Err(CurveError::TooFewPoints);
        }
        for (i, pair) in points.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            if !a.temp.0.is_finite() || !b.temp.0.is_finite() {
                return Err(CurveError::NonFinite);
            }
            if b.temp.0 <= a.temp.0 {
                return Err(CurveError::TempNotIncreasing { index: i + 1 });
            }
            if b.duty < a.duty {
                return Err(CurveError::DutyDecreasing { index: i + 1 });
            }
        }
        Ok(FanCurve { points })
    }

    pub fn from_pairs(pairs: &[(f32, f32)]) -> Result<Self, CurveError> {
        let points = pairs.iter().map(|&(t, d)| CurvePoint::new(t, d)).collect::<Result<Vec<_>, _>>()?;
        FanCurve::new(points)
    }

    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// Duty at `temp`. Below the first point: first duty. Above the last: last duty.
    /// A non-finite temperature returns 100 %.
    pub fn duty_at(&self, temp: Celsius) -> Percent {
        let t = temp.0;
        if !t.is_finite() {
            return Percent::FULL;
        }
        // `new` guarantees at least two points.
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if t <= first.temp.0 {
            return first.duty;
        }
        if t >= last.temp.0 {
            return last.duty;
        }
        for pair in self.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t <= b.temp.0 {
                let frac = (t - a.temp.0) / (b.temp.0 - a.temp.0);
                return Percent::clamped(a.duty.get() + frac * (b.duty.get() - a.duty.get()));
            }
        }
        last.duty
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FanTuning {
    /// The fan only slows down once the temperature has fallen this far below the point that
    /// would justify the current speed. Stops the fan hunting around a curve knee.
    pub hysteresis: Celsius,
    /// Max decrease in duty per second.
    pub ramp_down_per_sec: f32,
    /// Max increase in duty per second. `None` means jump straight to the target.
    pub ramp_up_per_sec: Option<f32>,
    /// At or above this, run at 100 % regardless of the curve.
    pub critical_temp: Celsius,
    /// Consecutive failed temperature reads before handing back to firmware.
    pub sensor_fail_ticks: u32,
}

impl Default for FanTuning {
    fn default() -> Self {
        FanTuning {
            hysteresis: Celsius(3.0),
            ramp_down_per_sec: 10.0,
            ramp_up_per_sec: None,
            critical_temp: Celsius(90.0),
            sensor_fail_ticks: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FanCommand {
    Duty(Percent),
    FirmwareAuto,
}

/// Runs a [`FanCurve`] over time. Call [`FanController::update`] once per control tick.
#[derive(Debug, Clone)]
pub struct FanController {
    curve: FanCurve,
    tuning: FanTuning,
    current: Option<f32>,
    failures: u32,
}

impl FanController {
    pub fn new(curve: FanCurve, tuning: FanTuning) -> Self {
        FanController { curve, tuning, current: None, failures: 0 }
    }

    pub fn curve(&self) -> &FanCurve {
        &self.curve
    }

    pub fn update(&mut self, temp: Option<Celsius>, dt: Duration) -> FanCommand {
        let temp = match temp.filter(|t| t.0.is_finite()) {
            Some(t) => {
                self.failures = 0;
                t
            }
            None => {
                self.failures = self.failures.saturating_add(1);
                return match self.current {
                    Some(c) if self.failures < self.tuning.sensor_fail_ticks => {
                        FanCommand::Duty(Percent::clamped(c))
                    }
                    _ => {
                        self.current = None;
                        FanCommand::FirmwareAuto
                    }
                };
            }
        };

        if temp.0 >= self.tuning.critical_temp.0 {
            self.current = Some(100.0);
            return FanCommand::Duty(Percent::FULL);
        }

        let raw = self.curve.duty_at(temp).get();
        let desired = match self.current {
            None => raw,
            Some(c) if raw >= c => raw,
            Some(c) => {
                let with_hysteresis = self.curve.duty_at(Celsius(temp.0 + self.tuning.hysteresis.0)).get();
                c.min(with_hysteresis).max(raw)
            }
        };

        let secs = dt.as_secs_f32();
        let next = match self.current {
            None => desired,
            Some(c) if desired > c => match self.tuning.ramp_up_per_sec {
                None => desired,
                Some(rate) => (c + rate * secs).min(desired),
            },
            Some(c) => (c - self.tuning.ramp_down_per_sec * secs).max(desired),
        };

        let next = Percent::clamped(next);
        self.current = Some(next.get());
        FanCommand::Duty(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SEC: Duration = Duration::from_secs(1);

    fn curve() -> FanCurve {
        FanCurve::from_pairs(&[(40.0, 30.0), (60.0, 50.0), (80.0, 100.0)]).unwrap()
    }

    fn duty(cmd: FanCommand) -> f32 {
        match cmd {
            FanCommand::Duty(d) => d.get(),
            FanCommand::FirmwareAuto => panic!("expected a duty, got FirmwareAuto"),
        }
    }

    #[test]
    fn interpolates_and_clamps_ends() {
        let c = curve();
        assert_eq!(c.duty_at(Celsius(20.0)).get(), 30.0);
        assert_eq!(c.duty_at(Celsius(50.0)).get(), 40.0);
        assert_eq!(c.duty_at(Celsius(70.0)).get(), 75.0);
        assert_eq!(c.duty_at(Celsius(95.0)).get(), 100.0);
        assert_eq!(c.duty_at(Celsius(f32::NAN)).get(), 100.0);
    }

    #[test]
    fn rejects_bad_curves() {
        assert_eq!(FanCurve::from_pairs(&[(40.0, 30.0)]), Err(CurveError::TooFewPoints));
        assert_eq!(
            FanCurve::from_pairs(&[(40.0, 30.0), (40.0, 50.0)]),
            Err(CurveError::TempNotIncreasing { index: 1 })
        );
        assert_eq!(
            FanCurve::from_pairs(&[(40.0, 60.0), (60.0, 50.0)]),
            Err(CurveError::DutyDecreasing { index: 1 })
        );
        assert_eq!(
            FanCurve::from_pairs(&[(40.0, 30.0), (60.0, 120.0)]),
            Err(CurveError::DutyOutOfRange(120.0))
        );
        assert_eq!(FanCurve::from_pairs(&[(f32::INFINITY, 30.0), (60.0, 50.0)]), Err(CurveError::NonFinite));
    }

    #[test]
    fn hysteresis_holds_speed_on_small_drops() {
        let mut fc = FanController::new(curve(), FanTuning::default());
        assert_eq!(duty(fc.update(Some(Celsius(60.0)), SEC)), 50.0);
        // A 2 °C drop is inside the 3 °C hysteresis band: hold 50 %.
        assert_eq!(duty(fc.update(Some(Celsius(58.0)), SEC)), 50.0);
        // A 10 °C drop: target is the curve at 50+3 = 53 °C = 43 %, limited to -10 %/s.
        assert_eq!(duty(fc.update(Some(Celsius(50.0)), SEC)), 43.0);
    }

    #[test]
    fn ramp_down_is_rate_limited() {
        let mut fc = FanController::new(curve(), FanTuning::default());
        assert_eq!(duty(fc.update(Some(Celsius(80.0)), SEC)), 100.0);
        assert_eq!(duty(fc.update(Some(Celsius(30.0)), SEC)), 90.0);
        assert_eq!(duty(fc.update(Some(Celsius(30.0)), SEC)), 80.0);
    }

    #[test]
    fn ramp_up_is_instant_by_default() {
        let mut fc = FanController::new(curve(), FanTuning::default());
        assert_eq!(duty(fc.update(Some(Celsius(30.0)), SEC)), 30.0);
        assert_eq!(duty(fc.update(Some(Celsius(80.0)), SEC)), 100.0);
    }

    #[test]
    fn critical_temp_forces_full_speed() {
        let tuning = FanTuning { critical_temp: Celsius(85.0), ..FanTuning::default() };
        let flat = FanCurve::from_pairs(&[(0.0, 30.0), (100.0, 30.0)]).unwrap();
        let mut fc = FanController::new(flat, tuning);
        assert_eq!(duty(fc.update(Some(Celsius(84.0)), SEC)), 30.0);
        assert_eq!(duty(fc.update(Some(Celsius(85.0)), SEC)), 100.0);
    }

    #[test]
    fn sensor_failure_hands_back_to_firmware() {
        let mut fc = FanController::new(curve(), FanTuning::default());
        assert_eq!(duty(fc.update(Some(Celsius(60.0)), SEC)), 50.0);
        assert_eq!(duty(fc.update(None, SEC)), 50.0);
        assert_eq!(duty(fc.update(Some(Celsius(f32::NAN)), SEC)), 50.0);
        assert_eq!(fc.update(None, SEC), FanCommand::FirmwareAuto);
        // Recovery: the next good reading takes control again.
        assert_eq!(duty(fc.update(Some(Celsius(60.0)), SEC)), 50.0);
    }

    #[test]
    fn sensor_failure_before_first_reading_is_firmware() {
        let mut fc = FanController::new(curve(), FanTuning::default());
        assert_eq!(fc.update(None, SEC), FanCommand::FirmwareAuto);
    }

    fn arb_curve() -> impl Strategy<Value = FanCurve> {
        prop::collection::vec((1.0f32..20.0, 0.0f32..20.0), 2..8).prop_map(|steps| {
            let mut t = 20.0;
            let mut d = 0.0;
            let pts: Vec<(f32, f32)> = steps
                .into_iter()
                .map(|(dt, dd)| {
                    t += dt;
                    d = (d + dd).min(100.0);
                    (t, d)
                })
                .collect();
            FanCurve::from_pairs(&pts).unwrap()
        })
    }

    proptest! {
        #[test]
        fn output_in_range_and_never_below_curve(
            curve in arb_curve(),
            temps in prop::collection::vec(prop::option::of(0.0f32..110.0), 1..60),
            dt_ms in 50u64..3000,
        ) {
            let tuning = FanTuning::default();
            let critical = tuning.critical_temp.0;
            let mut fc = FanController::new(curve.clone(), tuning);
            for t in temps {
                let cmd = fc.update(t.map(Celsius), Duration::from_millis(dt_ms));
                if let (FanCommand::Duty(d), Some(t)) = (cmd, t) {
                    prop_assert!((0.0..=100.0).contains(&d.get()));
                    prop_assert!(d.get() + 1e-3 >= curve.duty_at(Celsius(t)).get());
                    if t >= critical {
                        prop_assert_eq!(d.get(), 100.0);
                    }
                }
            }
        }

        #[test]
        fn steady_temperature_converges_to_curve(
            curve in arb_curve(),
            start in 0.0f32..89.0,
            end in 0.0f32..89.0,
        ) {
            let mut fc = FanController::new(curve.clone(), FanTuning { hysteresis: Celsius(0.0), ..FanTuning::default() });
            fc.update(Some(Celsius(start)), SEC);
            let mut last = 0.0;
            for _ in 0..20 {
                last = duty(fc.update(Some(Celsius(end)), SEC));
            }
            prop_assert!((last - curve.duty_at(Celsius(end)).get()).abs() < 1e-3);
        }
    }
}
