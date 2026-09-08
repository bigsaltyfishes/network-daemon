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

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub enum GlobalDaemonResponse {
    /// Acknowledgment of connection establishment
    Established,
    /// Acknowledgment of shutdown command
    ShutdownAck,
    /// WiFi Interface not found
    WiFiInterfaceNotFound { iface: String },
    /// General error response
    Error { message: String },
    /// Current or newly applied system hostname.
    Hostname(String),
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
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
    /// Read the system hostname.
    GetHostname,
    /// Apply a new system hostname.
    SetHostname { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_actions_round_trip() {
        let command = DaemonCommand::Global {
            action: GlobalDaemonAction::SetHostname {
                name: "router".to_string(),
            },
        };
        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: DaemonCommand = serde_json::from_str(&encoded).unwrap();
        assert!(matches!(
            decoded,
            DaemonCommand::Global {
                action: GlobalDaemonAction::SetHostname { .. }
            }
        ));
    }
}
