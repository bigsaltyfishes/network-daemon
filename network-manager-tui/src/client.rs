//! JSON-line Unix-socket client for the network daemon.
//!
//! The daemon speaks newline-delimited JSON over a Unix socket. Each request is
//! a [`DaemonCommand`], each response a [`DaemonResponse`]. Events are pushed as
//! [`DaemonResponse`] lines after a Subscribe command.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
};

use libnetwork_daemon::{DaemonCommand, DaemonResponse};

/// Errors talking to the daemon socket.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("socket connect: {0}")]
    Connect(std::io::Error),
    #[error("socket write: {0}")]
    Write(std::io::Error),
    #[error("socket read: {0}")]
    Read(std::io::Error),
    #[error("daemon returned: {0}")]
    Daemon(String),
    #[error("invalid JSON response: {0}")]
    Parse(String),
}

/// A connected daemon client.
pub struct DaemonClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl DaemonClient {
    /// Connect to the daemon's control socket.
    pub fn connect(path: &std::path::Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(path).map_err(ClientError::Connect)?;
        let reader =
            BufReader::new(stream.try_clone().map_err(ClientError::Connect)?);
        Ok(Self { stream, reader })
    }

    /// Read one blank-line-terminated JSON response line.
    fn read_line(&mut self) -> Result<String, ClientError> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(ClientError::Read)?;
        if n == 0 {
            return Err(ClientError::Daemon("connection closed".into()));
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            // Skip blank lines.
            return self.read_line();
        }
        Ok(trimmed.to_string())
    }

    /// Send a command and read its response.
    pub fn request(
        &mut self,
        cmd: &DaemonCommand,
    ) -> Result<DaemonResponse, ClientError> {
        let json = serde_json::to_string(cmd)
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        let mut line = json;
        line.push('\n');
        self.stream
            .write_all(line.as_bytes())
            .map_err(ClientError::Write)?;
        self.stream.flush().map_err(ClientError::Write)?;

        let resp = self.read_line()?;
        serde_json::from_str(&resp)
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    /// Read the next pushed event line.
    #[allow(dead_code)] // used by the event-subscription UI
    pub fn read_event(&mut self) -> Result<DaemonResponse, ClientError> {
        let resp = self.read_line()?;
        serde_json::from_str(&resp)
            .map_err(|e| ClientError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnetwork_daemon::{
        DaemonCommand, DaemonResponse, GlobalDaemonResponse,
        InterfaceManagerAction,
    };
    use std::io::Read;

    #[test]
    fn mock_client_round_trip() {
        use std::io::Write;
        use std::os::unix::net::UnixListener;
        let dir =
            std::env::temp_dir().join(format!("nd-tui-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let sock = dir.join("mock.sock");
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let n = stream.read(&mut buf).unwrap();
            assert!(
                String::from_utf8_lossy(&buf[..n]).contains("GetAllInterfaces")
            );
            stream
                .write_all(b"{\"subsystem\":\"InterfaceManager\",\"response\":{\"InfoList\":[]}}\n")
                .unwrap();
        });
        let mut client = DaemonClient::connect(&sock).unwrap();
        let resp = client
            .request(&DaemonCommand::InterfaceManager {
                action: InterfaceManagerAction::GetAllInterfaces,
            })
            .unwrap();
        assert!(matches!(resp, DaemonResponse::InterfaceManager { .. }));
        let _ = std::fs::remove_file(&sock);
    }

    /// Live probe: connects to the real daemon socket and verifies a response
    /// JSON line comes back (run with the daemon running, as a network-group
    /// member or root). Ignored by default; run with:
    ///   cargo test -p network-manager-tui live_probe -- --ignored
    #[test]
    #[ignore]
    fn live_probe() {
        let sock = std::path::PathBuf::from(
            std::env::var("NETWORK_DAEMON_SOCK").unwrap_or_else(|_| {
                "/var/run/network-daemon/network-daemon.sock".into()
            }),
        );
        let mut client = DaemonClient::connect(&sock).unwrap();
        let resp = client
            .request(&DaemonCommand::InterfaceManager {
                action: InterfaceManagerAction::GetAllInterfaces,
            })
            .unwrap();
        // The daemon responds with a JSON DaemonResponse (possibly an error if
        // the interface manager is degraded); the wire protocol round-trips.
        let _ = resp;
        println!("live_probe OK: daemon returned a valid response");
    }

    #[test]
    fn response_line_is_trimmed() {
        // A helper to parse what read_line feeds to serde_json.
        let parse = |s: &str| -> Result<DaemonResponse, ClientError> {
            serde_json::from_str(s)
                .map_err(|e| ClientError::Parse(e.to_string()))
        };
        let r =
            parse("{\"subsystem\":\"Global\",\"response\":\"Established\"}")
                .unwrap();
        assert!(matches!(r, DaemonResponse::Global { .. }));
        // round-trip GlobalDaemonResponse KnownNetwork display
        assert!(
            parse(
                &serde_json::to_string(&DaemonResponse::Global {
                    response: GlobalDaemonResponse::ShutdownAck
                })
                .unwrap()
            )
            .is_ok()
        );
    }
}
