//! `sol`: command-line client.
//!
//! Most commands talk to `sol-service` over its named pipe. `probe`, `doctor` and
//! `profile check` work without the service; `selftest` opens the hardware directly.

mod direct;
mod format;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sol_core::DeviceId;
use sol_core::profile::{FanSetting, Profile};
use sol_core::units::Percent;
use sol_ipc::{PROTOCOL_VERSION, Request, Response};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "sol", version, about = "Speed of Light: GPU tuning, fan control and monitoring")]
struct Cli {
    /// Named pipe of the service.
    #[arg(long, global = true, default_value = sol_ipc::PIPE_NAME)]
    pipe: String,
    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List backends and GPUs with capabilities and one telemetry sample (read-only, no service).
    Probe {
        #[arg(long, value_name = "N")]
        simulate: Option<u32>,
    },
    /// Write a hardware report for bug reports and the compatibility matrix (read-only).
    Doctor {
        #[arg(long, value_name = "N")]
        simulate: Option<u32>,
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Exercise each control on one GPU, verify it, and restore it. Needs --write and admin.
    Selftest {
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        write: bool,
        #[arg(long)]
        ignore_conflicts: bool,
        #[arg(long, value_name = "N")]
        simulate: Option<u32>,
    },
    /// Profile utilities.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Status of every GPU the service manages.
    Status,
    /// Live telemetry until Ctrl+C.
    Watch {
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },
    /// Apply a profile file.
    Apply {
        file: PathBuf,
        #[arg(long)]
        allow_unverified: bool,
    },
    /// Return GPUs to stock settings.
    Reset {
        #[arg(long)]
        device: Option<String>,
    },
    /// Set fans to firmware control or a fixed duty.
    Fan {
        #[command(subcommand)]
        mode: FanCommand,
        #[arg(long, global = true)]
        device: Option<String>,
        #[arg(long, global = true)]
        allow_unverified: bool,
    },
    /// Inspect or clear the crash guard.
    Guard {
        #[command(subcommand)]
        command: Option<GuardCommand>,
    },
}

#[derive(Subcommand, Debug)]
enum ProfileCommand {
    /// Validate a profile without applying it.
    Check { file: PathBuf },
}

#[derive(Subcommand, Debug)]
enum FanCommand {
    Auto,
    Fixed { duty: f32 },
}

#[derive(Subcommand, Debug)]
enum GuardCommand {
    Status,
    Clear,
}

fn read_profile(path: &PathBuf) -> Result<Profile> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Profile::from_toml(&text)?)
}

async fn connect(
    pipe: &str,
) -> Result<sol_ipc::framing::Client<tokio::net::windows::named_pipe::NamedPipeClient>> {
    sol_ipc::pipe::connect(pipe).await.with_context(|| {
        // Only the default pipe is the installed service's; a custom one is a console session.
        if pipe == sol_ipc::PIPE_NAME {
            format!("can't reach the service on {pipe}: {}", sol_service::winsvc::installed_state().explain())
        } else {
            format!("can't reach the service on {pipe}. Is `sol-service run --pipe {pipe}` running?")
        }
    })
}

async fn call(pipe: &str, req: Request) -> Result<Response> {
    let mut client = connect(pipe).await?;
    match client.call(&Request::Hello { protocol: PROTOCOL_VERSION }).await? {
        Response::Hello { .. } => {}
        Response::Error { message } => bail!("{message}"),
        other => bail!("unexpected handshake reply: {other:?}"),
    }
    Ok(client.call(&req).await?)
}

fn print_response(resp: &Response, json: bool) -> Result<i32> {
    if json {
        println!("{}", serde_json::to_string_pretty(resp)?);
    }
    let code = match resp {
        Response::Applied { reports } => {
            if !json {
                if reports.is_empty() {
                    println!("no device matched");
                }
                for r in reports {
                    print!("{}", format::report(r));
                }
            }
            if reports.iter().all(|r| r.all_applied()) && !reports.is_empty() { 0 } else { 2 }
        }
        Response::Status { devices, guard, backends } => {
            if !json {
                if let Some(line) = format::backends_line(backends, devices.is_empty()) {
                    println!("{line}\n");
                }
                if let Some(g) = guard {
                    println!(
                        "! crash guard tripped by profile {:?}: startup profile skipped. `sol guard clear` to reset.\n",
                        g.profile
                    );
                }
                for d in devices {
                    println!("{}", format::status_block(d));
                }
            }
            0
        }
        Response::Guard { tripped } => {
            if !json {
                match tripped {
                    Some(g) => {
                        println!("tripped by profile {:?} (armed at unix {})", g.profile, g.armed_at_unix)
                    }
                    None => println!("clear"),
                }
            }
            0
        }
        Response::Ok { message } => {
            if !json {
                println!("{message}");
            }
            0
        }
        Response::Error { message } => {
            if !json {
                eprintln!("error: {message}");
            }
            1
        }
        Response::Hello { .. } => 0,
    };
    Ok(code)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let json = cli.json;
    let req = match cli.command {
        Command::Probe { simulate } => return direct::probe(simulate, json),
        Command::Doctor { simulate, out } => return direct::doctor(simulate, out),
        Command::Selftest { device, write, ignore_conflicts, simulate } => {
            return direct::selftest(simulate, device, write, ignore_conflicts, json);
        }
        Command::Profile { command: ProfileCommand::Check { file } } => {
            let p = read_profile(&file)?;
            println!("{}: ok ({})", file.display(), p.name);
            return Ok(());
        }
        Command::Watch { interval_ms } => {
            let mut client = connect(&cli.pipe).await?;
            client.call(&Request::Hello { protocol: PROTOCOL_VERSION }).await?;
            let mut tick = tokio::time::interval(Duration::from_millis(interval_ms.max(100)));
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                }
                if let Response::Status { devices, .. } = client.call(&Request::Status).await? {
                    for d in devices {
                        match &d.telemetry {
                            Some(t) if json => println!("{}", serde_json::to_string(t)?),
                            Some(t) => println!("{}", format::telemetry_line(&d.info.name, t)),
                            None => println!(
                                "{} | {}",
                                d.info.name,
                                d.telemetry_error.as_deref().unwrap_or("no data")
                            ),
                        }
                    }
                }
            }
        }
        Command::Status => Request::Status,
        Command::Apply { file, allow_unverified } => {
            Request::ApplyProfile { profile: read_profile(&file)?, allow_unverified }
        }
        Command::Reset { device } => Request::Reset { device: device.map(DeviceId) },
        Command::Fan { mode, device, allow_unverified } => Request::SetFan {
            device: device.map(DeviceId),
            setting: match mode {
                FanCommand::Auto => FanSetting::Auto,
                FanCommand::Fixed { duty } => FanSetting::Fixed { duty: Percent::new(duty)? },
            },
            allow_unverified,
        },
        Command::Guard { command } => match command {
            None | Some(GuardCommand::Status) => Request::GuardStatus,
            Some(GuardCommand::Clear) => Request::GuardClear,
        },
    };
    let resp = call(&cli.pipe, req).await?;
    let code = print_response(&resp, json)?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
