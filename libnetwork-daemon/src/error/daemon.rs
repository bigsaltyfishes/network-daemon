use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

use crate::error::{InterfaceError, WifiError, WpaCtrlError};

#[derive(Debug, Error, Serialize, JsonSchema)]
#[serde(tag = "error_type", content = "details")]
pub enum NetworkDaemonError {
    /// Interface manager error
    #[error("interface manager error: {0}")]
    InterfaceManagerError(#[from] InterfaceError),
    /// WiFi manager error
    #[error("wifi manager error: {0}")]
    WifiManagerError(#[from] WifiError),
    /// WPA control error
    #[error("wpa control error: {0}")]
    WpaCtrlError(#[from] WpaCtrlError),
    /// Invalid parameter error
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
    /// Client Writer Closed
    #[error("Client writer closed")]
    ClientWriterClosed,
}

impl From<async_channel::SendError<String>> for NetworkDaemonError {
    fn from(_err: async_channel::SendError<String>) -> Self {
        NetworkDaemonError::ClientWriterClosed
    }
}
