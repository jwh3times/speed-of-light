//! Named-pipe transport (Windows).
//!
//! Access control: the pipe's DACL grants full access to SYSTEM and Administrators, and
//! read/write to *interactive* users, so the unelevated GUI or CLI of the person at the
//! keyboard can drive the elevated service. Network logons and services running as other
//! accounts can't connect, and remote clients are rejected outright. See SECURITY.md for the
//! threat model and what is still open (server identity verification).

use crate::framing::{Client, Handler, serve_connection};
use std::ffi::c_void;
use std::io;
use std::time::Duration;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};
use tokio::sync::watch;
use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

/// D:P = protected DACL. SY = LocalSystem, BA = Builtin Administrators, IU = Interactive Users.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

fn create_server(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let sddl: Vec<u16> = PIPE_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated UTF-16 and `sd` is a valid out-pointer.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    // SAFETY: `sa` points at a valid SECURITY_ATTRIBUTES whose descriptor outlives this call.
    let result = unsafe {
        ServerOptions::new()
            // Fail if someone else already owns this name, rather than serving next to a squatter.
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, &mut sa as *mut SECURITY_ATTRIBUTES as *mut c_void)
    };
    // SAFETY: `sd` was allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(sd) };
    result
}

/// Accept connections until `shutdown` flips to `true`.
pub async fn serve(name: &str, handler: Handler, mut shutdown: watch::Receiver<bool>) -> io::Result<()> {
    let mut server = create_server(name, true)?;
    loop {
        tokio::select! {
            r = server.connect() => r?,
            _ = shutdown.changed() => return Ok(()),
        }
        let connected = std::mem::replace(&mut server, create_server(name, false)?);
        let h = handler.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_connection(connected, h).await {
                tracing::debug!(error = %e, "client connection ended with an error");
            }
        });
    }
}

/// Connect to the service, waiting briefly if every pipe instance is busy.
pub async fn connect(name: &str) -> io::Result<Client<NamedPipeClient>> {
    let mut waited = Duration::ZERO;
    loop {
        match ClientOptions::new().open(name) {
            Ok(c) => return Ok(Client::new(c)),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) && waited < Duration::from_secs(2) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                waited += Duration::from_millis(50);
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PROTOCOL_VERSION, Request, Response};
    use std::sync::Arc;

    #[tokio::test]
    async fn round_trip_over_a_real_pipe_and_refuses_a_second_server() {
        let name = format!(r"\\.\pipe\speed-of-light-test-{}", std::process::id());
        let handler: Handler = Arc::new(|req| match req {
            Request::Hello { protocol } => Response::Hello { protocol, service_version: "t".into() },
            _ => Response::Ok { message: String::new() },
        });
        let (tx, rx) = watch::channel(false);
        let server = tokio::spawn({
            let name = name.clone();
            async move { serve(&name, handler, rx).await }
        });
        // Wait for the server to create the pipe.
        let mut client = loop {
            match connect(&name).await {
                Ok(c) => break c,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };
        let r = client.call(&Request::Hello { protocol: PROTOCOL_VERSION }).await.unwrap();
        assert!(matches!(r, Response::Hello { .. }));

        // A second "first instance" on the same name must fail: no squatting.
        assert!(create_server(&name, true).is_err());

        // Several clients can be connected at once.
        let mut second = connect(&name).await.unwrap();
        assert!(matches!(second.call(&Request::Status).await.unwrap(), Response::Ok { .. }));

        tx.send(true).unwrap();
        server.await.unwrap().unwrap();
    }
}
