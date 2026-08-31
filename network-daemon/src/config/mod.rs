//! TOML configuration (non-sensitive daemon settings).
//!
//! This module owns the user-editable `config.toml`: daemon behaviour and
//! non-sensitive state (known-network metadata, per-interface settings).
//! Sensitive Wi-Fi credentials (PSK / EAP passwords) are deliberately NOT in
//! this file; they live encrypted in the `StorageManager`'s SQLite store.

use std::{collections::BTreeMap, path::Path};

use thiserror::Error;

/// Per-interface persisted settings (non-sensitive).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct InterfaceConfig {
    /// Accept kernel Router Advertisements (SLAAC) for IPv6.
    #[serde(default)]
    pub slaac: bool,
    /// Run the internal DHCPv4 client.
    #[serde(default = "default_true")]
    pub dhcpv4: bool,
}

fn default_true() -> bool {
    true
}

/// Known-network metadata. Credentials live in SQLite, never here.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkConfig {
    pub ssid: String,
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
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(ConfigError::Read)?;
        std::fs::rename(&tmp, path).map_err(ConfigError::Read)
    }

    /// Insert or update the metadata for a known network (by ssid/bssid).
    pub fn upsert_network(&mut self, net: NetworkConfig) {
        if let Some(existing) = self
            .networks
            .iter_mut()
            .find(|n| n.ssid == net.ssid && n.bssid == net.bssid)
        {
            *existing = net;
        } else {
            self.networks.push(net);
        }
    }

    /// Remove the metadata for a known network. Returns whether it existed.
    pub fn remove_network(&mut self, ssid: &str, bssid: Option<&str>) -> bool {
        let before = self.networks.len();
        self.networks
            .retain(|n| !(n.ssid == ssid && n.bssid.as_deref() == bssid));
        self.networks.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
