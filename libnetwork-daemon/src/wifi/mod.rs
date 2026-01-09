mod events;
mod link;
mod network;
mod security;
mod status;

use async_channel::Receiver;
pub use events::*;
pub use link::{CountryCode, RegDomain, WlanMode};
pub use network::{KnownNetwork, KnownNetworkState, ScanResult};
use schemars::JsonSchema;
pub use security::Security;
use serde::{Deserialize, Serialize};
pub use status::{SupplicantStatus, WpaState};

use crate::{MacAddr, error::WifiError};

#[derive(Debug, Serialize, JsonSchema)]
pub enum WiFiManagerResponse {
    ScanResults(Vec<ScanResult>),
    KnownNetworks(Vec<KnownNetwork>),
    Status(SupplicantStatus),
    Success(()),
    Event(WiFiManagerEvent),
    #[serde(skip)]
    EventReceiver(Receiver<WiFiManagerEvent>),
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WlanLinkOptions {
    pub regdomain: RegDomain,
    pub region: CountryCode,
    pub mode: WlanMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum WiFiManagerEvent {
    /// New scan results are available
    ScanResultsAvailable,
    /// Supplicant status updated
    StatusUpdated {
        iface: String,
        status: SupplicantStatus,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command")]
pub enum WiFiManagerAction {
    /// Start a scan
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    ///
    /// # Errors
    ///
    /// - `WifiError::ScanFailed`: Scan command failed
    Scan,
    /// Fetch current cached scan results
    ///
    /// # Returns
    ///
    /// List of cached scan results
    ScanResults,
    /// Fetch known/saved networks
    ///
    /// # Returns
    ///
    /// List of known networks
    ///
    /// # Errors
    ///
    /// - `WifiError::GetKnownNetworksFailed`: Failed to get known networks
    KnownNetworks,
    /// Fetch current supplicant status
    ///
    /// # Returns
    ///
    /// Current supplicant status
    ///
    /// # Errors
    ///
    /// - `WifiError::GetStatusFailed`: Failed to get status
    Status,
    /// Register a known network (does not connect)
    ///
    /// # Arguments
    ///
    /// - `ssid`: Network SSID
    /// - `bssid`: Optional BSSID to lock on
    /// - `security`: Security type
    /// - `password`: Password (if applicable)
    /// - `identity`: Identity (if applicable)
    /// - `hidden`: Whether the network is hidden
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    ///
    /// # Errors
    ///
    /// - `WifiError::AddNetworkFailed`: Failed to register network
    AddNetwork {
        ssid: String,
        bssid: Option<MacAddr>,
        security: Security,
        password: Option<String>,
        identity: Option<String>,
        #[serde(default)]
        hidden: bool,
    },
    /// Remove a network
    ///
    /// # Arguments
    ///
    /// - `ssid`: Network SSID
    /// - `bssid`: Optional BSSID to remove (exact match required)
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    ///
    /// # Errors
    ///
    /// - `WifiError::KnownNetworkNotFound`: Network entry not registered
    /// - `WifiError::RemoveNetworkFailed`: Failed to remove network from
    ///   supplicant
    RemoveNetwork {
        ssid: String,
        bssid: Option<MacAddr>,
    },
    /// Connect to a stored known network
    ///
    /// # Arguments
    ///
    /// - `ssid`: Network SSID
    /// - `bssid`: Optional BSSID to connect to; when omitted, the strongest
    ///   known BSSID will be chosen from current scan results
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    ///
    /// # Errors
    ///
    /// - `WifiError::KnownNetworkNotFound`: Network is not registered
    /// - `WifiError::BssidRequired`: SSID has no known BSSID and caller did not
    ///   provide one
    /// - `WifiError::ConnectFailed`: Failed to connect
    Connect {
        ssid: String,
        bssid: Option<MacAddr>,
    },
    /// Disconnect from current network
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    ///
    /// # Errors
    ///
    /// - `WifiError::DisconnectFailed`: Failed to disconnect
    Disconnect,
    /// Reconnect to current network
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    ///
    /// # Errors
    ///
    /// - `WifiError::ConnectFailed`: Failed to reconnect
    Reconnect,
    /// Subscribe to WiFi events
    SubscribeEvents,
    /// Internal event from wpa_supplicant
    #[serde(skip)]
    WpaEvent { event: WpaEvent },
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub enum WpaCommand {
    Bss { addr: String },
    Ping,
    Scan,
    ScanResults,
    Status,
    Set { key: String, value: String },
    AddNetwork,
    SetNetwork { id: i32, key: String, value: String },
    GetNetwork { id: i32, key: String },
    ListNetworks,
    EnableNetwork { id: i32 },
    DisableNetwork { id: i32 },
    RemoveNetwork { id: i32 },
    SelectNetwork { id: Option<i32> },
    Reassociate,
    Reconfigure,
    Disconnect,
    Reconnect,
    Attach,
    Detach,
    SaveConfig,
    Unknown(String),
}

impl WpaCommand {
    /// Convert command to string
    pub fn to_string(&self) -> String {
        match self {
            WpaCommand::Bss { addr } => format!("BSS {}", addr),
            WpaCommand::Ping => "PING".to_string(),
            WpaCommand::Scan => "SCAN".to_string(),
            WpaCommand::ScanResults => "SCAN_RESULTS".to_string(),
            WpaCommand::Status => "STATUS".to_string(),
            WpaCommand::Set { key, value } => format!("SET {} {}", key, value),
            WpaCommand::AddNetwork => "ADD_NETWORK".to_string(),
            WpaCommand::SetNetwork { id, key, value } => {
                format!("SET_NETWORK {} {} {}", id, key, value)
            }
            WpaCommand::GetNetwork { id, key } => {
                format!("GET_NETWORK {} {}", id, key)
            }
            WpaCommand::ListNetworks => "LIST_NETWORKS".to_string(),
            WpaCommand::EnableNetwork { id } => {
                format!("ENABLE_NETWORK {}", id)
            }
            WpaCommand::DisableNetwork { id } => {
                format!("DISABLE_NETWORK {}", id)
            }
            WpaCommand::RemoveNetwork { id } => {
                format!("REMOVE_NETWORK {}", id)
            }
            WpaCommand::SelectNetwork { id } => match id {
                Some(id) => format!("SELECT_NETWORK {}", id),
                None => "SELECT_NETWORK any".to_string(),
            },
            WpaCommand::Reassociate => "REASSOCIATE".to_string(),
            WpaCommand::Reconfigure => "RECONFIGURE".to_string(),
            WpaCommand::Disconnect => "DISCONNECT".to_string(),
            WpaCommand::Reconnect => "RECONNECT".to_string(),
            WpaCommand::Attach => "ATTACH".to_string(),
            WpaCommand::Detach => "DETACH".to_string(),
            WpaCommand::SaveConfig => "SAVE_CONFIG".to_string(),
            WpaCommand::Unknown(cmd) => cmd.clone(),
        }
    }
}

impl AsRef<WpaCommand> for WpaCommand {
    fn as_ref(&self) -> &WpaCommand {
        self
    }
}
