# Safety

Speed of Light changes power, clocks and fans on hardware that costs hundreds or thousands of
dollars. This page lists what the code guarantees, and which test fails if a guarantee breaks.
Change an invariant only alongside its test.

## Writes

| Invariant | Where | Guarded by |
| --- | --- | --- |
| Out-of-range values are rejected before anything is written (driver range and user policy) | `tuner::plan` | `tuner::tests::out_of_range_values_are_rejected_before_writing`, `policy_caps_apply_inside_the_driver_range` |
| Writes on an unverified driver/arch are blocked unless the user opts in | `tuner::compat_gate` | `tuner::tests::unverified_drivers_are_blocked_by_default`, `service::unverified_writes_are_blocked_unless_the_request_opts_in` |
| Known-broken combinations are blocked even with the opt-in | `compat::lookup` (broken beats verified) | `tuner::tests::known_broken_is_blocked_even_when_unverified_is_allowed`, `compat::tests::broken_beats_verified_and_ranges_are_inclusive` |
| Every write is read back; accepted-but-ignored writes are reported as `NotEnforced` | `tuner::write_and_verify` | `tuner::tests::accepted_but_ignored_writes_are_not_enforced` |
| Returning to stock is never blocked by the matrix | `tuner::reset` | `tuner::tests::reset_restores_stock_and_verifies_it`, `service::reset_returns_to_stock` |
| Profiles with unknown keys are rejected (a typo must not silently do nothing) | `profile` (`deny_unknown_fields`) | `profile::tests::rejects_typos` |

## Fans

| Invariant | Where | Guarded by |
| --- | --- | --- |
| Duty is always in 0..=100; a non-finite value becomes 100 % | `units::Percent` | `units::tests::*` |
| Curves must be monotonic (duty never falls as temperature rises) | `FanCurve::new` | `fan::tests::rejects_bad_curves` |
| Output is never below the curve at the current temperature | `FanController::update` | property test `output_in_range_and_never_below_curve` |
| At or above the critical temperature, 100 % immediately | `FanController::update` | `critical_temp_forces_full_speed`, property test |
| A curve's critical temp is capped at the firmware slowdown point | `tuner::plan` | `tuner::tests::curve_critical_temp_is_capped_at_firmware_slowdown` |
| Sensor lost for N ticks: hand back to firmware | `FanController::update` + service | `fan::tests::sensor_failure_hands_back_to_firmware`, `service::curve_hands_back_to_firmware_when_the_sensor_dies` |
| Repeated failed fan writes: hand back to firmware | service tick | `service::curve_hands_back_to_firmware_after_repeated_write_failures` |
| A fixed duty is overridden before thermal slowdown | service tick (`FIXED_FAN_WATCHDOG_MARGIN`) | `service::a_too_quiet_fixed_fan_is_handed_back_to_firmware_before_slowdown` |
| A curve actually keeps a loaded GPU cool (closed-loop, simulated physics) | service + sim | `service::fan_curve_keeps_a_loaded_gpu_out_of_thermal_slowdown` |
| Requested duty below the driver minimum is *raised*, never lowered | `FanCaps::clamp` | `tuner::tests::fixed_fan_is_raised_to_driver_minimum` |
| Service start takes fans back to firmware (a previous instance may have died mid-curve) | `Service::new` | `service::startup_takes_fans_back_from_a_dead_previous_instance` |
| Graceful stop hands fans back to firmware | `Service::shutdown` | `service::graceful_shutdown_returns_fans_and_disarms_the_guard` |

The safety tests were mutation-checked. Disabling the fixed-fan watchdog, or letting
hysteresis drop below the curve, makes them fail.

## Crash guard

A profile that touches power or clocks arms a sentinel file in the state directory. It
disarms after the stability window (default 120 s), or on graceful shutdown. If the sentinel
is present at start, the startup profile is skipped until `sol guard clear`. Guarded by
`service::crash_guard_blocks_the_startup_profile_after_a_crash` and
`service::guard_disarms_once_the_stability_window_passes`. An unreadable sentinel counts as
tripped (`crashguard::tests::corrupt_sentinel_still_counts_as_armed`).

## Known gaps

These are tracked in ROADMAP.md:

- **Process killed, not stopped.** If `sol-service` is killed (Task Manager, crash) while a curve
  or fixed duty is active, the fans stay at the last duty until the service restarts or the
  driver resets. The GPU's own firmware thermal protection (slowdown/shutdown) still applies.
  The fix is to run as a Windows service with automatic restart, so the startup takeover kicks
  in within seconds.
- **Clock offsets are not stability-tested.** The tool verifies that the driver applied an
  offset, not that the GPU is stable at it. A built-in stability test is on the roadmap.
- **Pipe server identity.** Clients don't yet verify that the process on the pipe is the real
  service. See SECURITY.md.
