//! WPA Supplicant control interface error types

use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

use crate::{WpaCommand, error::IoError};

#[derive(Debug, Error, Serialize, JsonSchema)]
pub enum WpaSocketError {
    #[error("Socket creation failed: {0}")]
    SocketCreationFailed(#[source] IoError),
    #[error("Socket bind failed: {0}")]
    BindFailed(#[source] IoError),
    #[error("Socket connect failed: {0}")]
    ConnectFailed(#[source] IoError),
    #[error("Send failed: {0}")]
    SendFailed(#[source] IoError),
    #[error("Receive failed: {0}")]
    RecvFailed(#[source] IoError),
    #[error("Invalid socket path")]
    InvalidPath,
}

/// Errors that can occur when communicating with wpa_supplicant
#[derive(Error, Debug, Serialize, JsonSchema)]
pub enum WpaCtrlError {
    /// Failed to send command
    #[error("failed to send command: {cmd:?}")]
    SendFailed {
        cmd: WpaCommand,
        #[source]
        source: WpaSocketError,
    },

    /// Socket Error
    #[error("socket error: {0}")]
    SocketError(#[from] WpaSocketError),

    /// Io error
    #[error("I/O error: {0}")]
    IoError(#[from] IoError),

    /// Request timed out
    #[error("request timed out")]
    Timeout,

    /// ATTACH command failed
    #[error("ATTACH command failed")]
    AttachFailed,

    /// DETACH command failed
    #[error("DETACH command failed")]
    DetachFailed,

    /// wpa_supplicant returned an error
    #[error("wpa_supplicant returned error: {0}")]
    CommandFailed(String),

    /// Failed to parse response
    #[error("failed to parse response: {0}")]
    ParseError(String),

    /// Failed to wait for event
    #[error("failed to wait for event: {event}")]
    WaitEventFailed { event: String },

    /// No default control interface found
    #[error("no default wpa_supplicant control interface found")]
    NoDefaultPath,
}
