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

use crate::MacAddr;

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
        status: Box<SupplicantStatus>,
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

impl std::fmt::Display for WpaCommand {
    /// Render the command as the wire string sent to wpa_supplicant.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WpaCommand::Bss { addr } => write!(f, "BSS {}", addr),
            WpaCommand::Ping => write!(f, "PING"),
            WpaCommand::Scan => write!(f, "SCAN"),
            WpaCommand::ScanResults => write!(f, "SCAN_RESULTS"),
            WpaCommand::Status => write!(f, "STATUS"),
            WpaCommand::Set { key, value } => write!(f, "SET {} {}", key, value),
            WpaCommand::AddNetwork => write!(f, "ADD_NETWORK"),
            WpaCommand::SetNetwork { id, key, value } => {
                write!(f, "SET_NETWORK {} {} {}", id, key, value)
            }
            WpaCommand::GetNetwork { id, key } => {
                write!(f, "GET_NETWORK {} {}", id, key)
            }
            WpaCommand::ListNetworks => write!(f, "LIST_NETWORKS"),
            WpaCommand::EnableNetwork { id } => write!(f, "ENABLE_NETWORK {}", id),
            WpaCommand::DisableNetwork { id } => write!(f, "DISABLE_NETWORK {}", id),
            WpaCommand::RemoveNetwork { id } => write!(f, "REMOVE_NETWORK {}", id),
            WpaCommand::SelectNetwork { id } => match id {
                Some(id) => write!(f, "SELECT_NETWORK {}", id),
                None => write!(f, "SELECT_NETWORK any"),
            },
            WpaCommand::Reassociate => write!(f, "REASSOCIATE"),
            WpaCommand::Reconfigure => write!(f, "RECONFIGURE"),
            WpaCommand::Disconnect => write!(f, "DISCONNECT"),
            WpaCommand::Reconnect => write!(f, "RECONNECT"),
            WpaCommand::Attach => write!(f, "ATTACH"),
            WpaCommand::Detach => write!(f, "DETACH"),
            WpaCommand::SaveConfig => write!(f, "SAVE_CONFIG"),
            WpaCommand::Unknown(cmd) => write!(f, "{}", cmd),
        }
    }
}

impl AsRef<WpaCommand> for WpaCommand {
    fn as_ref(&self) -> &WpaCommand {
        self
    }
}
