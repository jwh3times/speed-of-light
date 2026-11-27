//! `sol`'s text output through a pipe is UTF-8, so the degree sign is `C2 B0`. A reader that
//! decodes it with another code page shows `┬░C` (#38): that is the reader's setting, and the
//! README tells scripts to decode as UTF-8 or use `--json`. This guards the bytes `sol` writes.

#![allow(clippy::expect_used)]

use std::process::{Command, Stdio};

#[test]
fn piped_text_output_writes_the_degree_sign_as_utf8() {
    let out = Command::new(env!("CARGO_BIN_EXE_sol"))
        .args(["probe", "--simulate", "1"])
        .stdout(Stdio::piped())
        .output()
        .expect("run sol");
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).expect("stdout is valid UTF-8");
    assert!(stdout.contains("slowdown 88 \u{b0}C"), "{stdout}");
}
