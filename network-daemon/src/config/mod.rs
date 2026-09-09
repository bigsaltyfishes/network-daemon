//! TOML configuration (non-sensitive daemon settings).
//!
//! This module owns the user-editable `config.toml`: daemon behaviour and
//! non-sensitive state (known-network metadata, per-interface settings).
//! Sensitive Wi-Fi credentials (PSK / EAP passwords) are deliberately NOT in
//! this file; they live encrypted in the `StorageManager`'s SQLite store.

use std::{collections::BTreeMap, path::Path};

use libnetwork_daemon::{LinkOptions, WlanLinkOptions};
use thiserror::Error;

/// Per-interface persisted settings (non-sensitive).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InterfaceConfig {
    /// Accept kernel Router Advertisements (SLAAC) for IPv6.
    #[serde(default)]
    pub slaac: bool,
    /// Run the internal DHCPv4 client.
    #[serde(default = "default_true")]
    pub dhcpv4: bool,
    /// Allow automatic WLAN interface creation for this device.
    ///
    /// This defaults to true for backwards compatibility. Setting it to
    /// false gives the device to an external interface manager.
    #[serde(default = "default_true")]
    pub create_wlan: bool,
    /// Explicit WLAN creation parameters. Their presence opts this device
    /// out of the daemon's default automatic creation path.
    #[serde(default)]
    pub wlan: Option<WlanLinkOptions>,
    /// Explicit logical-interface creation parameters. When present, the
    /// daemon recreates the link during interface refresh if it is missing.
    #[serde(default)]
    pub creation: Option<LinkOptions>,
}

impl Default for InterfaceConfig {
    fn default() -> Self {
        Self {
            slaac: false,
            dhcpv4: true,
            create_wlan: true,
            wlan: None,
            creation: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Known-network metadata. Credentials live in SQLite, never here.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkConfig {
    pub ssid: String,
    /// MAC address of the WLAN interface that owns this saved AP.
    /// `None` keeps compatibility with pre-MAC configuration entries.
    #[serde(default)]
    pub interface_mac: Option<String>,
    /// Optional BSSID lock (MAC string). Credentials are keyed by this too.
    #[serde(default)]
    pub bssid: Option<String>,
    /// Security variant name: "Open" | "Psk" | "Eap" | "Unknown".
    #[serde(default)]
    pub security: String,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Daemon configuration loaded from a TOML file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DaemonConfig {
    /// Known-network metadata (credentials in SQLite).
    #[serde(default)]
    pub networks: Vec<NetworkConfig>,
    /// Per-interface settings keyed by interface name.
    #[serde(default)]
    pub interface: BTreeMap<String, InterfaceConfig>,
}

/// Errors while reading the TOML configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {0}")]
    NotFound(String),
    #[error("failed to read config file: {0}")]
    Read(std::io::Error),
    #[error("failed to parse config file: {0}")]
    Parse(String),
}

impl DaemonConfig {
    /// Load and parse the configuration file.
    ///
    /// A missing file is an error (the daemon must not silently run without
    /// its configuration); the caller decides whether to write a default.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    ConfigError::NotFound(path.display().to_string())
                }
                _ => ConfigError::Read(e),
            })?;
        Self::parse(&text)
    }

    /// Parse configuration from TOML text.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Atomically persist the configuration to `path` (temp + rename).
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string(self)
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(ConfigError::Read)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(ConfigError::Read)?;
        std::fs::rename(&tmp, path).map_err(ConfigError::Read)
    }

    /// Insert or update metadata for one interface-owned AP profile.
    pub fn upsert_network(&mut self, net: NetworkConfig) {
        if let Some(existing) = self.networks.iter_mut().find(|n| {
            n.interface_mac == net.interface_mac
                && n.ssid == net.ssid
                && n.bssid == net.bssid
                && n.security == net.security
        }) {
            *existing = net;
        } else {
            self.networks.push(net);
        }
    }

    /// Remove the metadata for a known network. Returns whether it existed.
    pub fn remove_network(
        &mut self,
        interface_mac: Option<&str>,
        ssid: &str,
        bssid: Option<&str>,
        security: Option<&str>,
    ) -> bool {
        let before = self.networks.len();
        self.networks.retain(|n| {
            let same = n.interface_mac.as_deref() == interface_mac
                && n.ssid == ssid
                && n.bssid.as_deref() == bssid
                && security.is_none_or(|value| n.security == value);
            !same
        });
        self.networks.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnetwork_daemon::{CountryCode, RegDomain, WlanMode};

    #[test]
    fn parse_empty_is_default() {
        let cfg = DaemonConfig::parse("").unwrap();
        assert!(cfg.networks.is_empty());
        assert!(cfg.interface.is_empty());
    }

    #[test]
    fn parse_network_metadata() {
        let text = r#"
[[networks]]
ssid = "HomeWiFi"
security = "Psk"
hidden = true
priority = 5
"#;
        let cfg = DaemonConfig::parse(text).unwrap();
        assert_eq!(cfg.networks.len(), 1);
        let n = &cfg.networks[0];
        assert_eq!(n.ssid, "HomeWiFi");
        assert_eq!(n.security, "Psk");
        assert!(n.hidden);
        assert_eq!(n.priority, 5);
        assert!(n.enabled); // default true
        assert!(n.bssid.is_none());
        assert!(n.interface_mac.is_none());
    }

    #[test]
    fn parse_interface_config_with_defaults() {
        let text = r#"
[interface.eth0]
slaac = true
"#;
        let cfg = DaemonConfig::parse(text).unwrap();
        let ifc = &cfg.interface["eth0"];
        assert!(ifc.slaac);
        assert!(ifc.dhcpv4); // default true
        assert!(ifc.create_wlan); // default true
        assert!(ifc.wlan.is_none());
        assert!(ifc.creation.is_none());
    }

    #[test]
    fn parse_wireless_creation_policy() {
        let text = r#"
[interface.iwn0]
create_wlan = false

[interface.ath0.wlan]
regdomain = "Fcc"
region = "US"
mode = "Sta"
"#;
        let cfg = DaemonConfig::parse(text).unwrap();
        assert!(!cfg.interface["iwn0"].create_wlan);
        assert!(cfg.interface["iwn0"].wlan.is_none());
        let options = cfg.interface["ath0"].wlan.as_ref().unwrap();
        assert_eq!(options.regdomain, RegDomain::Fcc);
        assert_eq!(options.region, CountryCode::US);
        assert_eq!(options.mode, WlanMode::Sta);
    }

    #[test]
    fn parse_logical_interface_creation() {
        let text = r#"
[interface.bridge0.creation]
type = "Bridge"
members = ["eth0", "eth1"]

[interface.vlan10.creation]
type = "Vlan"
parent = "eth0"
tag = 10

[interface.lagg0.creation]
type = "Lagg"
protocol = "lacp"
members = ["eth0", "eth1"]
"#;
        let cfg = DaemonConfig::parse(text).unwrap();
        assert!(matches!(
            cfg.interface["bridge0"].creation,
            Some(LinkOptions::Bridge { ref members })
                if members == &["eth0".to_string(), "eth1".to_string()]
        ));
        assert!(matches!(
            cfg.interface["vlan10"].creation,
            Some(LinkOptions::Vlan { ref parent, tag })
                if parent == "eth0" && tag == 10
        ));
        assert!(matches!(
            cfg.interface["lagg0"].creation,
            Some(LinkOptions::Lagg { protocol, ref members })
                if protocol == libnetwork_daemon::LaggProtocol::Lacp
                    && members.len() == 2
        ));

        let encoded = toml::to_string(&cfg).unwrap();
        let decoded = DaemonConfig::parse(&encoded).unwrap();
        assert!(matches!(
            decoded.interface["vlan10"].creation,
            Some(LinkOptions::Vlan { ref parent, tag })
                if parent == "eth0" && tag == 10
        ));
    }

    #[test]
    fn parse_invalid_toml_is_error() {
        assert!(DaemonConfig::parse("not = = valid").is_err());
    }

    #[test]
    fn round_trip_serde() {
        let text = r#"
[[networks]]
ssid = "X"
security = "Open"
"#;
        let cfg = DaemonConfig::parse(text).unwrap();
        let out = toml::to_string(&cfg).unwrap();
        let cfg2: DaemonConfig = toml::from_str(&out).unwrap();
        assert_eq!(cfg2.networks[0].ssid, "X");
    }
}
