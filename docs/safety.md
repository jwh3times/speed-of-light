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
| Uninstall returns every GPU to stock, and says so when it can't | `reset_to_stock` | `service::uninstall_returns_every_gpu_to_stock`, `service::uninstall_reports_a_gpu_it_could_not_reset` |
| Uninstalling a `--simulate` install writes to no real GPU, whichever version registered it; any other (or unreadable) install is reset | `registration::uninstall_resets_real_gpus` | `registration::tests::uninstalling_a_simulated_install_touches_no_real_gpu` |
| Policy caps bind the startup profile and every client's request, and no request loosens them; a setting over a cap is refused, not lowered | `ServiceOptions::policy`, used by `Service::apply` | `service::policy_caps_bind_the_startup_profile_and_every_request` |
| A service config with an unknown key or an out-of-range value is rejected | `config::ServiceConfig::from_toml` | `config::tests::a_config_that_is_wrong_is_rejected_naming_what_is_wrong`, `config::tests::install_refuses_a_service_config_that_is_not_valid` |
| A rejected service config (untrusted, unreadable or invalid) blocks every write except a return to stock, and never falls back to the defaults' missing caps ([ADR 0003](adr/0003-a-bad-service-config-blocks-every-write.md)) | `config::load_installed` + `Service::handle` / `Service::startup` | `service::a_rejected_service_config_blocks_every_write_except_a_return_to_stock`, `config::tests::a_service_config_others_could_change_is_rejected_even_when_it_is_valid`, `config::tests::an_invalid_service_config_is_rejected_naming_the_file_and_the_problem` |
| The installed service applies no startup profile a non-administrator could have changed | `trust::untrusted_reason` + `Service::startup` | `trust::tests::a_file_users_can_write_is_exposed`, `trust::tests::a_profile_owned_by_a_standard_user_is_not_trusted_even_if_its_entries_are_locked_down`, `trust::tests::a_file_in_a_folder_users_can_write_is_exposed_even_if_the_file_is_locked_down`, `trust::tests::a_profile_only_administrators_can_change_is_trusted`, `service::a_rejected_startup_profile_is_not_applied_and_a_notice_says_why` |
| `install` takes the state folder back from whoever created it | `trust::harden_dir` | `trust::tests::hardening_a_folder_protects_it_and_clears_explicit_entries_below_it` (the owner change needs elevation: checked by the [acceptance run](testing.md#service-acceptance-run-manual-elevated-simulated-gpus)) |
| The startup profile waits for a GPU that is ready, and nothing touches a GPU before it is found | `Service::new` / `Service::tick` | `service::the_startup_profile_waits_for_a_gpu_that_is_not_ready_yet` |
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
| Graceful stop hands fans back to firmware, whether it comes from Ctrl+C, a Windows stop request or system shutdown | `Service::shutdown`, reached from `host::run` | `service::graceful_shutdown_returns_fans_and_disarms_the_guard` (the Windows stop path: [acceptance run](testing.md#service-acceptance-run-manual-elevated-simulated-gpus)) |
| A killed service is restarted within seconds, and its startup takeover hands the fans back | `registration::registration` (recovery actions) | `registration::tests::a_dead_service_is_restarted_within_seconds_every_time` |
| Another tuning tool that starts later is reported, and the service keeps driving its fans | `Service::tick` | `service::a_tuning_tool_started_after_the_service_gets_a_notice_and_fan_writes_continue`, `service::a_tuning_tool_seen_before_the_gpu_is_ready_is_reported_on_the_gpu_found_later` |
| A console `run` doesn't open real GPUs while the installed service owns them (its startup takeover would pull the fans off the service's curve) | `winsvc::console_may_open_real_gpus` | `winsvc::tests::a_console_run_refuses_real_gpus_while_the_installed_service_owns_them` |

The safety tests were mutation-checked. Disabling the fixed-fan watchdog, or letting
hysteresis drop below the curve, makes them fail.

## Crash guard

A profile that touches power or clocks arms a sentinel file in the state directory. It
disarms after the stability window (default 120 s), or on graceful shutdown. If the sentinel
is present at start, the startup profile is skipped until `sol guard clear`. That includes a
restart after the service itself was killed inside the window: it can't tell that from a PC
crash, and skipping fails safe. Guarded by
`service::crash_guard_blocks_the_startup_profile_after_a_crash` and
`service::guard_disarms_once_the_stability_window_passes`. An unreadable sentinel counts as
tripped (`crashguard::tests::corrupt_sentinel_still_counts_as_armed`).

## Known gaps

These are tracked in ROADMAP.md, or in the issue named:

- **Killed between restarts.** If the installed service is killed while a curve or fixed duty is
  active, the fans stay at the last duty for the seconds until Windows restarts it (2 s, 2 s,
  then 10 s for each later failure) and its startup takeover hands them back. A console
  `sol-service run` isn't restarted at all. The GPU's own firmware thermal protection
  (slowdown/shutdown) still applies throughout.
- **GPU lost while running.** A driver update or reset under a running service isn't recovered
  yet (#32).
- **Sleep and resume.** Nobody has checked yet whether power and clock settings survive sleep
  (#33).
- **Clock offsets are not stability-tested.** The tool verifies that the driver applied an
  offset, not that the GPU is stable at it. A built-in stability test is on the roadmap.
- **Pipe server identity.** Clients don't yet verify that the process on the pipe is the real
  service. See SECURITY.md.
