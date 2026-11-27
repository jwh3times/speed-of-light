//! `sol` when nothing serves the pipe: the explanation alone, without the raw
//! "cannot find the file" error that reads like a missing file (#39).

#![allow(clippy::expect_used)]

use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sol")).args(args).output().expect("run sol");
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn an_unserved_pipe_gets_the_explanation_without_the_raw_os_error() {
    let pipe = format!(r"\\.\pipe\speed-of-light-unserved-{}", std::process::id());
    for command in ["status", "watch"] {
        let (code, stderr) = run(&[command, "--pipe", &pipe]);
        assert_eq!(code, Some(1), "sol {command}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "can't reach the service on {pipe}. Is `sol-service run --pipe {pipe}` running?"
            )),
            "sol {command}: {stderr}"
        );
        assert!(!stderr.contains("Caused by"), "sol {command}: {stderr}");
        assert!(!stderr.contains("os error"), "sol {command}: {stderr}");
    }
}
