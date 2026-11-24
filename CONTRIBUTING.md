# Contributing

## The most valuable contribution: hardware reports

You don't need to write code. If you have a GPU, see "Help verify your GPU" in the README.
Every report on a new card, driver or laptop extends the compatibility matrix for everyone.

## Code

1. Read [AGENTS.md](AGENTS.md). It lists the rules every change follows. The short version:
   - all writes go through `sol_core::tuner`;
   - backends stay thin;
   - missing sensors are `None`;
   - fans fail towards cooling;
   - tests never write to real hardware.
2. `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace` must pass. CI runs exactly these.
3. A new safety behaviour needs a test that fails without it, and a row in
   [docs/safety.md](docs/safety.md).
4. A new backend implements `sol_core::GpuDevice`. Its tests run against fakes, plus an
   `#[ignore]`d read-only live probe.

## License of contributions

This project is licensed under [AGPL-3.0-only](LICENSE). By opening a pull request, you agree
that your contribution is licensed under the same terms (inbound = outbound). Sign off your
commits (`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org/).

## Compatibility matrix changes

Each new entry needs evidence: a hardware-report issue link, or `sol selftest` output in the
PR. See [docs/compat-matrix.md](docs/compat-matrix.md).
