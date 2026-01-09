use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    InterfaceManagerAction, InterfaceResponse, WiFiManagerAction,
    WiFiManagerResponse, error::NetworkDaemonError,
};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "subsystem")]
pub enum DaemonCommand {
    InterfaceManager {
        action: InterfaceManagerAction,
    },
    WiFiManager {
        iface: String,
        action: WiFiManagerAction,
    },
    Global {
        action: GlobalDaemonAction,
    },
}

#[derive(Debug, Serialize, JsonSchema)]
pub enum GlobalDaemonResponse {
    /// Acknowledgment of connection establishment
    Established,
    /// Acknowledgment of shutdown command
    ShutdownAck,
    /// WiFi Interface not found
    WiFiInterfaceNotFound { iface: String },
    /// General error response
    Error { message: String },
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "subsystem")]
pub enum DaemonResponse {
    InterfaceManager {
        response: InterfaceResponse,
    },
    WiFiManager {
        iface: String,
        response: WiFiManagerResponse,
    },
    Global {
        response: GlobalDaemonResponse,
    },
    Error(NetworkDaemonError),
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command")]
pub enum GlobalDaemonAction {
    /// Shutdown current connection
    Shutdown,
}
