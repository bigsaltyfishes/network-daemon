mod mac;
mod state;

use async_channel::Receiver;
pub use mac::MacAddr;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use state::{ConnectedState, ConnectionState};

use crate::{
    Lease, PrefixedIpAddr, PrefixedIpv4Addr, PrefixedIpv6Addr, WlanLinkOptions,
};

#[derive(Debug, Serialize, JsonSchema)]
pub enum InterfaceResponse {
    Success(()),
    Info(InterfaceInfo),
    InfoList(Vec<InterfaceInfo>),
    Event(InterfaceManagerEvent),
    #[serde(skip)]
    EventReceiver(Receiver<InterfaceManagerEvent>),
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum LinkOptions {
    Wlan {
        parent: String,
        #[serde(default)]
        options: WlanLinkOptions,
    },
}

#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(tag = "action", content = "value")]
pub enum Modification<T> {
    /// Update or Add the value
    Append(T),
    /// Remove the value
    Remove(T),
    /// Replace with the value
    Replace(T),
    /// Clear all values
    Clear,
    /// No change
    #[default]
    NoChange,
}

/// Actions for the InterfaceManager
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command")]
pub enum InterfaceManagerAction {
    /// Force refresh all interface information
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    ForceRefresh,
    /// Get information about a specific interface
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Info(InterfaceInfo)` if found, or
    /// `InterfaceResponse::Error(InterfaceError::NotFound)` if not found
    GetInterfaceInfo { name: String },
    /// Update interface information based on an event
    ///
    /// This event should only be sent by internal components
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    #[serde(skip)]
    UpdateInterfaceInfo { event: InterfaceManagerEvent },
    /// Update interface state for interface that state managed externally
    ///
    /// This event should only be sent by internal components
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    #[serde(skip)]
    UpdateExternalInterfaceState {
        name: String,
        state: ConnectionState,
    },
    /// Mark an interface as unmanaged
    ///
    /// This event should only be sent by internal components
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    #[serde(skip)]
    MarkAsUnManaged { name: String },
    /// Get information about all interfaces
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::InfoList(Vec<InterfaceInfo>)` on completion
    GetAllInterfaces,
    /// Get interfaces by type
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::InfoList(Vec<InterfaceInfo>)` on completion
    GetInterfacesByType { interface_type: InterfaceType },
    /// Get interfaces by parent interface name
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::InfoList(Vec<InterfaceInfo>)` on completion
    GetInterfacesByParent { parent: String },
    /// Get interfaces by connection state
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::InfoList(Vec<InterfaceInfo>)` on completion
    GetInterfacesByState { state: ConnectionState },
    /// Subscribe to interface manager events
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::EventReceiver(Receiver<InterfaceManagerEvent>)` on completion
    SubscribeEvents,
    /// Add a new interface (Bridge or Wlan)
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    AddLink {
        name: String,
        kind: InterfaceType,
        /// Required for Wlan
        options: Option<LinkOptions>,
    },
    /// Delete an interface
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    DelLink { name: String },
    /// Modify an interface (IPs, Up/Down)
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    ModLink {
        name: String,
        #[serde(default)]
        ipv4: Modification<PrefixedIpv4Addr>,
        #[serde(default)]
        ipv6: Modification<PrefixedIpv6Addr>,
        #[serde(default)]
        oper_state: Modification<bool>,
        #[serde(default)]
        slaac: Modification<bool>,
    },
    /// DHCP Set event for an interface
    ///
    /// This event should only be sent by internal components
    ///
    /// # Response
    ///
    /// Sends `InterfaceResponse::Success(())` on completion
    #[serde(skip)]
    DhcpV4Set {
        name: String,
        old_lease: Option<Lease>,
        new_lease: Option<Lease>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum InterfaceManagerEvent {
    InterfaceAdded(InterfaceInfo),
    InterfaceRemoved(InterfaceInfo),
    InterfaceChanged(InterfaceInfo),
}

/// Type of network interface
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
pub enum InterfaceType {
    Bridge,
    Tun,
    Vlan,
    Vxlan,
    GreTun,
    Wireguard,
    Loopback,
    Wlan,
    Ethernet,
    Lagg,
    Usbus,
    Tap,
    Vmnet,
    Openvpn,
    Stf,
    Epair,
    Enc,
    Pflog,
    Pfsync,
    Ipfw,
    Ipfwlog,
    Disc,
    Me,
    Edsc,
    Ipsec,
    Gif,
    #[default]
    Other,
}

/// Information about a network interface
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct InterfaceInfo {
    /// Interface ID
    /// This is typically the index assigned by the OS
    pub id: u32,
    /// Interface name (e.g., "wlan0")
    pub name: String,
    /// Connection state
    pub state: ConnectionState,
    /// Interface type
    pub interface_type: InterfaceType,
    /// MAC address
    pub mac_addr: Option<MacAddr>,
    /// IPv4 addresses and their prefix lengths
    pub ipv4_addrs: Vec<PrefixedIpv4Addr>,
    /// IPv6 address (first non-link-local) and its prefix length
    pub ipv6_addrs: Vec<PrefixedIpv6Addr>,
    /// IPv4 gateway address
    pub gateway_ipv4: Option<PrefixedIpv4Addr>,
    /// IPv6 gateway address
    pub gateway_ipv6: Option<PrefixedIpv6Addr>,
    /// DHCPv4 enable flag
    pub dhcpv4_enabled: bool,
    /// SLAAC (kernel Router Advertisement) enabled flag
    ///
    /// When enabled, IPv6 is configured by the FreeBSD kernel's SLAAC rather
    /// than by the daemon.
    #[serde(default)]
    pub slaac_enabled: bool,
    /// Parent interface (for wlan devices)
    pub parent: Option<String>,
}

impl InterfaceInfo {
    /// Create a new InterfaceInfo with just a name
    pub fn new<T>(id: u32, name: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            id,
            name: name.into(),
            state: ConnectionState::NotApplicable,
            interface_type: InterfaceType::Other,
            mac_addr: None,
            ipv4_addrs: Vec::new(),
            ipv6_addrs: Vec::new(),
            gateway_ipv4: None,
            gateway_ipv6: None,
            dhcpv4_enabled: true,
            slaac_enabled: false,
            parent: None,
        }
    }

    /// Add an IP address to the interface
    pub fn add_addr(&mut self, addr: PrefixedIpAddr) {
        match addr {
            PrefixedIpAddr::V4(v4) => self.ipv4_addrs.push(v4),
            PrefixedIpAddr::V6(v6) => self.ipv6_addrs.push(v6),
        }
    }

    /// Contains IP address
    pub fn contains_addr(&self, addr: &PrefixedIpAddr) -> bool {
        match addr {
            PrefixedIpAddr::V4(v4) => self.ipv4_addrs.contains(v4),
            PrefixedIpAddr::V6(v6) => self.ipv6_addrs.contains(v6),
        }
    }

    /// Check if this is a WLAN interface
    pub fn is_wlan(&self) -> bool {
        self.interface_type == InterfaceType::Wlan
    }
}

impl Default for InterfaceInfo {
    fn default() -> Self {
        Self::new(0, "")
    }
}
