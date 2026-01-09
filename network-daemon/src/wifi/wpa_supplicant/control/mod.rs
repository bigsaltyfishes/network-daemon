mod events;
mod socket;

use std::{path::Path, time::Duration};

use async_io::Async;
pub use events::*;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use libnetwork_daemon::{
    WpaCommand,
    error::{WpaCtrlError, WpaSocketError},
};
use socket::WpaSocket;

/// Default maximum reply size from wpa_supplicant
pub const WPA_MAX_REPLY_SIZE: usize = 4096;

/// Default timeout for wpa_supplicant requests
pub const WPA_DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for scan operations
pub const WPA_SCAN_TIMEOUT: Duration = Duration::from_secs(30);

/// WPA Supplicant control interface
///
/// Provides async communication with wpa_supplicant through Unix domain
/// sockets. The connection is established to a specific interface's control
/// socket (e.g., `/var/run/wpa_supplicant/wlan0`).
///
/// # Clone
///
/// Cloning this struct creates a new reference to the same underlying socket.
pub struct WpaCtrl(Async<WpaSocket>);

impl WpaCtrl {
    pub async fn open<P, R>(
        bind_path: P,
        ctrl_path: R,
    ) -> Result<Self, WpaCtrlError>
    where
        P: AsRef<Path> + Send,
        R: AsRef<Path> + Send,
    {
        let ctrl_path = ctrl_path.as_ref();
        let local_path = bind_path.as_ref();

        // Remove existing socket file if present
        let _ = tokio::fs::remove_file(&local_path).await;

        // Create and bind local socket
        let socket = WpaSocket::connect(&local_path, ctrl_path).await?;

        Ok(Self(
            Async::new(socket).map_err(|e| WpaCtrlError::IoError(e.into()))?,
        ))
    }

    pub async fn request<C>(&mut self, cmd: C) -> Result<String, WpaCtrlError>
    where
        C: AsRef<WpaCommand> + Send + Sync,
    {
        let cmd = cmd.as_ref();

        // Send command
        self.0
            .write_all(cmd.to_string().as_bytes())
            .await
            .map_err(|e| WpaCtrlError::SendFailed {
                cmd: cmd.clone(),
                source: WpaSocketError::SendFailed(e.into()),
            })?;

        // Receive response with timeout
        let mut buf = vec![0u8; WPA_MAX_REPLY_SIZE];

        let len = self
            .0
            .read(&mut buf)
            .await
            .map_err(|e| WpaSocketError::RecvFailed(e.into()))?;

        // Convert to string, trimming any trailing whitespace/null
        let response = String::from_utf8_lossy(&buf[..len])
            .trim_end_matches(|c: char| c.is_whitespace() || c == '\0')
            .to_string();

        Ok(response)
    }

    pub async fn request_ok<C>(&mut self, cmd: C) -> Result<(), WpaCtrlError>
    where
        C: AsRef<WpaCommand> + Clone + Send + Sync,
    {
        let response = self.request(cmd.clone()).await?;

        if response == "OK" {
            Ok(())
        } else if response == "FAIL" {
            Err(WpaCtrlError::CommandFailed(format!(
                "command '{:?}' failed",
                cmd.as_ref()
            )))
        } else {
            Err(WpaCtrlError::CommandFailed(format!(
                "unexpected response to '{:?}': {}",
                cmd.as_ref(),
                response
            )))
        }
    }

    pub async fn recv(&mut self) -> Result<Option<String>, WpaCtrlError> {
        let mut buf = vec![0u8; WPA_MAX_REPLY_SIZE];
        let len = self
            .0
            .read(&mut buf)
            .await
            .map_err(|e| WpaSocketError::RecvFailed(e.into()))?;

        if len == 0 {
            return Ok(None);
        } else {
            let response =
                String::from_utf8_lossy(&buf[..len]).trim_end().to_string();
            Ok(Some(response))
        }
    }
}
