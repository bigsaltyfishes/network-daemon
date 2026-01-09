//! WiFi security types

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// WiFi security type
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
pub enum Security {
    /// Open network (no encryption)
    Open,
    /// EAP (enterprise authentication)
    Eap,
    /// PSK (WPA/WPA2-Personal)
    Psk,
    /// Unknown security type
    #[default]
    Unknown,
}

impl fmt::Display for Security {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "Open"),
            Self::Eap => write!(f, "EAP"),
            Self::Psk => write!(f, "PSK"),
            Self::Unknown => write!(f, "N/A"),
        }
    }
}

impl Security {
    /// Parse from wpa_supplicant flags string (from SCAN_RESULTS)
    ///
    /// # Arguments
    ///
    /// * `flags` - Flags string like "[WPA2-PSK-CCMP][ESS]"
    ///
    /// # Returns
    ///
    /// Detected security type
    ///
    /// # Example
    ///
    /// ```
    /// use libnetwork_daemon::Security;
    ///
    /// assert_eq!(Security::from_flags("[WPA2-PSK-CCMP][ESS]"), Security::Psk);
    /// assert_eq!(Security::from_flags("[WPA2-EAP-CCMP][ESS]"), Security::Eap);
    /// assert_eq!(Security::from_flags("[ESS]"), Security::Open);
    /// ```
    pub fn from_flags(flags: &str) -> Self {
        if flags.contains("PSK") {
            Self::Psk
        } else if flags.contains("EAP") || flags.contains("IEEE8021X") {
            Self::Eap
        } else if flags.contains("WEP") {
            // Treat WEP as PSK-like (requires password)
            Self::Psk
        } else {
            Self::Open
        }
    }

    /// Parse from wpa_supplicant key_mgmt string (from GET_NETWORK)
    ///
    /// # Arguments
    ///
    /// * `key_mgmt` - key_mgmt value like "WPA-PSK", "WPA-EAP", "NONE"
    ///
    /// # Returns
    ///
    /// Detected security type
    pub fn from_key_mgmt(key_mgmt: &str) -> Self {
        match key_mgmt.trim() {
            "NONE" => Self::Open,
            "WPA-EAP"
            | "IEEE8021X"
            | "WPA-EAP-SHA256"
            | "WPA-EAP-SUITE-B-192" => Self::Eap,
            s if s.contains("PSK") => Self::Psk,
            _ => Self::Unknown,
        }
    }

    /// Check if this security type requires a password
    pub fn requires_password(&self) -> bool {
        matches!(self, Self::Psk)
    }

    /// Check if this security type requires identity (username)
    pub fn requires_identity(&self) -> bool {
        matches!(self, Self::Eap)
    }

    /// Get a short description for display
    pub fn description(&self) -> &'static str {
        match self {
            Self::Open => "No password required",
            Self::Eap => "Enterprise authentication (username/password)",
            Self::Psk => "Password required (WPA/WPA2)",
            Self::Unknown => "Unknown security",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_flags() {
        assert_eq!(Security::from_flags("[WPA2-PSK-CCMP][ESS]"), Security::Psk);
        assert_eq!(Security::from_flags("[WPA-PSK-TKIP][ESS]"), Security::Psk);
        assert_eq!(Security::from_flags("[WPA2-EAP-CCMP][ESS]"), Security::Eap);
        assert_eq!(Security::from_flags("[ESS]"), Security::Open);
        assert_eq!(Security::from_flags(""), Security::Open);
        assert_eq!(Security::from_flags("[WEP]"), Security::Psk);
    }

    #[test]
    fn test_from_key_mgmt() {
        assert_eq!(Security::from_key_mgmt("WPA-PSK"), Security::Psk);
        assert_eq!(Security::from_key_mgmt("WPA2-PSK"), Security::Psk);
        assert_eq!(Security::from_key_mgmt("WPA-EAP"), Security::Eap);
        assert_eq!(Security::from_key_mgmt("IEEE8021X"), Security::Eap);
        assert_eq!(Security::from_key_mgmt("NONE"), Security::Open);
        assert_eq!(Security::from_key_mgmt("UNKNOWN"), Security::Unknown);
    }

    #[test]
    fn test_requirements() {
        assert!(Security::Psk.requires_password());
        assert!(!Security::Open.requires_password());
        assert!(!Security::Eap.requires_password());

        assert!(Security::Eap.requires_identity());
        assert!(!Security::Psk.requires_identity());
        assert!(!Security::Open.requires_identity());
    }

    #[test]
    fn test_display() {
        assert_eq!(Security::Psk.to_string(), "PSK");
        assert_eq!(Security::Eap.to_string(), "EAP");
        assert_eq!(Security::Open.to_string(), "Open");
        assert_eq!(Security::Unknown.to_string(), "N/A");
    }
}
