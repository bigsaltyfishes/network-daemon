use std::{
    io::{Read, Write},
    os::{
        fd::{AsFd, AsRawFd},
        unix::net::UnixDatagram,
    },
    path::{Path, PathBuf},
};

use async_io::IoSafe;
use libnetwork_daemon::error::WpaSocketError;

pub struct WpaSocket {
    /// Unix datagram socket
    socket: UnixDatagram,
    /// Temporary directory for local socket
    #[allow(dead_code)] // resource path held for the socket lifetime
    sock_dir: PathBuf,
    /// Local socket path
    local_path: PathBuf,
    /// Remote wpa_supplicant socket path
    #[allow(dead_code)] // resource path held for the socket lifetime
    remote_path: PathBuf,
}

impl WpaSocket {
    /// Connect to wpa_supplicant socket
    ///
    /// # Arguments
    ///
    /// * `bind` - Path to bind local socket
    /// * `remote` - Path to wpa_supplicant socket
    ///
    /// # Returns
    ///
    /// * `Ok(WpaSocket)` - Successfully connected
    /// * `Err(WpaSocketError)` - Connection failed
    pub async fn connect<B, R>(
        bind: B,
        remote: R,
    ) -> Result<Self, WpaSocketError>
    where
        B: AsRef<Path>,
        R: AsRef<Path>,
    {
        let bind = bind.as_ref();
        let remote = remote.as_ref();
        let sock_dir = bind
            .parent()
            .ok_or(WpaSocketError::InvalidPath)?
            .to_path_buf();

        // Create directory recursively
        tokio::fs::create_dir_all(&sock_dir)
            .await
            .map_err(|e| WpaSocketError::SocketCreationFailed(e.into()))?;

        // Create and bind local socket
        let socket = UnixDatagram::bind(bind)
            .map_err(|e| WpaSocketError::BindFailed(e.into()))?;

        // Connect to wpa_supplicant
        socket
            .connect(remote)
            .map_err(|e| WpaSocketError::ConnectFailed(e.into()))?;

        Ok(Self {
            socket,
            sock_dir,
            local_path: bind.to_path_buf(),
            remote_path: remote.to_path_buf(),
        })
    }

    /// Get reference to the underlying socket
    #[allow(dead_code)] // socket accessor
    pub fn socket(&self) -> &UnixDatagram {
        &self.socket
    }

    /// Get local socket path
    #[allow(dead_code)] // local socket path accessor
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }

    /// Get remote socket path
    #[allow(dead_code)] // remote socket path accessor
    pub fn remote_path(&self) -> &Path {
        &self.remote_path
    }

    /// Get the raw file descriptor
    #[allow(dead_code)] // raw fd accessor
    pub fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        self.socket.as_raw_fd()
    }
}

impl AsFd for WpaSocket {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.socket.as_fd()
    }
}

impl AsRawFd for WpaSocket {
    fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        self.socket.as_raw_fd()
    }
}

impl Read for WpaSocket {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        self.socket.recv(buf)
    }
}

impl Write for WpaSocket {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        self.socket.send(buf)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

impl Drop for WpaSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.local_path);
    }
}

unsafe impl IoSafe for WpaSocket {}
