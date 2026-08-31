//! WiFi operation error types

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{MacAddr, error::WpaCtrlError};

/// Errors that can occur during WiFi operations
#[derive(Error, Debug, Serialize, Deserialize, JsonSchema)]
pub enum WifiError {
    /// WPA control error
    #[error("WPA control error")]
    WpaCtrl(#[from] WpaCtrlError),

    /// Scan failed
    #[error("scan failed")]
    ScanFailed,

    /// Failed to get scan results
    #[error("failed to get scan results")]
    GetScanResultsFailed,

    /// SSID not available
    #[error("SSID not available: {ssid}")]
    SsidUnavailable { ssid: String },

    /// Known network not found for the given SSID/BSSID
    #[error("known network not found: ssid={ssid}, bssid={bssid:?}")]
    KnownNetworkNotFound {
        ssid: String,
        bssid: Option<MacAddr>,
    },

    /// BSSID is required to connect to this SSID
    #[error("bssid required for ssid: {ssid}")]
    BssidRequired { ssid: String },

    /// Failed to add network
    #[error("failed to add network")]
    AddNetworkFailed,

    /// Failed to configure network
    #[error("failed to configure network: network_id={nwid}")]
    ConfigureNetworkFailed { nwid: i32 },

    /// Failed to remove network
    #[error("failed to remove network: network_id={nwid}")]
    RemoveNetworkFailed { nwid: i32 },

    /// Failed to connect
    #[error("connection failed")]
    ConnectFailed,

    /// Failed to disconnect
    #[error("disconnect failed")]
    DisconnectFailed,

    /// Failed to save configuration
    #[error("failed to save configuration")]
    SaveConfigFailed,

    /// Invalid password length
    #[error(
        "invalid password length: must be between {min} and {max} characters"
    )]
    InvalidPasswordLength { min: usize, max: usize },

    /// Failed to parse response
    #[error("failed to parse response: {0}")]
    ParseError(String),

    /// Network not found
    #[error("network not found: id={nwid}")]
    NetworkNotFound { nwid: i32 },

    /// Operation not supported
    #[error("operation not supported: {0}")]
    NotSupported(String),

    /// WPA Supplicant already running
    #[error("WPA Supplicant already running for interface: {0}")]
    WpaSupplicantRunning(String),

    /// Failed to start WPA Supplicant
    #[error("failed to start WPA Supplicant: {0}")]
    SupplicantStartFailed(String),

    /// Other error
    #[error("{0}")]
    Other(String),
}

impl<T> From<T> for WifiError
where
    T: AsRef<str>,
{
    fn from(err: T) -> Self {
        WifiError::Other(err.as_ref().to_string())
    }
}
