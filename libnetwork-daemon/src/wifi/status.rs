//! Supplicant status

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{utils::unescape_ssid, wifi::Security};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
pub enum WpaState {
    Completed,
    Disconnected,
    Scanning,
    Associating,
    Associated,
    Authenticating,
    FourWayHandshake,
    GroupHandshake,
    Inactive,
    InterfaceDisabled,
    Unknown,
}

impl WpaState {
    /// Parse from string
    pub fn from_str(s: &str) -> Self {
        match s {
            "COMPLETED" => Self::Completed,
            "DISCONNECTED" => Self::Disconnected,
            "SCANNING" => Self::Scanning,
            "ASSOCIATING" => Self::Associating,
            "ASSOCIATED" => Self::Associated,
            "AUTHENTICATING" => Self::Authenticating,
            "4WAY_HANDSHAKE" => Self::FourWayHandshake,
            "GROUP_HANDSHAKE" => Self::GroupHandshake,
            "INACTIVE" => Self::Inactive,
            "INTERFACE_DISABLED" => Self::InterfaceDisabled,
            _ => Self::Unknown,
        }
    }
}

/// wpa_supplicant status
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SupplicantStatus {
    /// Current frequency in MHz
    pub freq: Option<i32>,
    /// WPA state (e.g., "COMPLETED", "SCANNING", "DISCONNECTED")
    pub state: Option<WpaState>,
    /// Current BSSID
    pub bssid: Option<String>,
    /// Current SSID
    pub ssid: Option<String>,
    /// IP address
    pub ip_address: Option<String>,
    /// Security type (key_mgmt)
    pub security: Option<Security>,
    /// Network ID
    pub id: Option<i32>,
    /// Mode (station, AP, etc.)
    pub mode: Option<String>,
    /// Pairwise cipher
    pub pairwise_cipher: Option<String>,
    /// Group cipher
    pub group_cipher: Option<String>,
    /// EAP identity (if using EAP)
    pub eap_identity: Option<String>,
}

impl SupplicantStatus {
    /// Parse from STATUS response
    ///
    /// # Arguments
    ///
    /// * `response` - Raw STATUS response from wpa_supplicant
    ///
    /// # Returns
    ///
    /// Parsed status structure
    ///
    /// # Example
    ///
    /// ```
    /// use libnetwork_daemon::SupplicantStatus;
    ///
    /// let response = "bssid=aa:bb:cc:dd:ee:ff
    /// freq=2412
    /// ssid=TestNetwork
    /// id=0
    /// mode=station
    /// pairwise_cipher=CCMP
    /// group_cipher=CCMP
    /// key_mgmt=WPA2-PSK
    /// wpa_state=COMPLETED
    /// ip_address=192.168.1.100
    /// ";
    ///
    /// let status = SupplicantStatus::parse(response);
    /// assert!(status.is_connected());
    /// assert_eq!(status.ssid, Some("TestNetwork".to_string()));
    /// ```
    pub fn parse(response: &str) -> Self {
        let mut status = Self::default();

        for line in response.lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "bssid" => status.bssid = Some(value.to_string()),
                    "freq" => status.freq = value.parse().ok(),
                    "ssid" => status.ssid = Some(unescape_ssid(value)),
                    "id" => status.id = value.parse().ok(),
                    "mode" => status.mode = Some(value.to_string()),
                    "pairwise_cipher" => {
                        status.pairwise_cipher = Some(value.to_string())
                    }
                    "group_cipher" => {
                        status.group_cipher = Some(value.to_string())
                    }
                    "key_mgmt" => {
                        status.security = Some(Security::from_key_mgmt(value))
                    }
                    "wpa_state" => {
                        status.state = Some(WpaState::from_str(value))
                    }
                    "ip_address" => status.ip_address = Some(value.to_string()),
                    "eap_session_id" | "eap_method" => {
                        // EAP is being used
                        if status.security.is_none() {
                            status.security = Some(Security::Eap);
                        }
                    }
                    "identity" => status.eap_identity = Some(value.to_string()),
                    _ => {}
                }
            }
        }

        status
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.state == Some(WpaState::Completed)
    }

    /// Check if scanning
    pub fn is_scanning(&self) -> bool {
        self.state == Some(WpaState::Scanning)
    }

    /// Check if disconnected
    pub fn is_disconnected(&self) -> bool {
        matches!(
            self.state,
            Some(WpaState::Disconnected) | Some(WpaState::Inactive) | None
        )
    }

    /// Check if associating (in progress)
    pub fn is_associating(&self) -> bool {
        matches!(
            self.state,
            Some(WpaState::Associating)
                | Some(WpaState::Associated)
                | Some(WpaState::FourWayHandshake)
                | Some(WpaState::GroupHandshake)
        )
    }

    /// Get human-readable state description
    pub fn state_description(&self) -> &str {
        match self.state {
            Some(WpaState::Completed) => "Connected",
            Some(WpaState::Disconnected) => "Disconnected",
            Some(WpaState::Scanning) => "Scanning",
            Some(WpaState::Associating) => "Associating",
            Some(WpaState::Associated) => "Associated",
            Some(WpaState::Authenticating) => "Authenticating",
            Some(WpaState::FourWayHandshake) => "Handshaking",
            Some(WpaState::GroupHandshake) => "Group Handshake",
            Some(WpaState::Inactive) => "Inactive",
            Some(WpaState::InterfaceDisabled) => "Interface Disabled",
            Some(WpaState::Unknown) | None => "Unknown",
        }
    }

    /// Get frequency band
    pub fn band(&self) -> Option<&'static str> {
        self.freq
            .map(|f| if f < 3000 { "2.4 GHz" } else { "5 GHz" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_connected_status() {
        let response = r#"bssid=aa:bb:cc:dd:ee:ff
freq=2412
ssid=TestNetwork
id=0
mode=station
pairwise_cipher=CCMP
group_cipher=CCMP
key_mgmt=WPA2-PSK
wpa_state=COMPLETED
ip_address=192.168.1.100"#;

        let status = SupplicantStatus::parse(response);

        assert!(status.is_connected());
        assert!(!status.is_disconnected());
        assert_eq!(status.bssid, Some("aa:bb:cc:dd:ee:ff".to_string()));
        assert_eq!(status.freq, Some(2412));
        assert_eq!(status.ssid, Some("TestNetwork".to_string()));
        assert_eq!(status.id, Some(0));
        assert_eq!(status.security, Some(Security::Psk));
        assert_eq!(status.ip_address, Some("192.168.1.100".to_string()));
        assert_eq!(status.state_description(), "Connected");
        assert_eq!(status.band(), Some("2.4 GHz"));
    }

    #[test]
    fn test_parse_disconnected_status() {
        let response = r#"wpa_state=DISCONNECTED"#;

        let status = SupplicantStatus::parse(response);

        assert!(!status.is_connected());
        assert!(status.is_disconnected());
        assert_eq!(status.state_description(), "Disconnected");
    }

    #[test]
    fn test_parse_scanning_status() {
        let response = r#"wpa_state=SCANNING"#;

        let status = SupplicantStatus::parse(response);

        assert!(status.is_scanning());
        assert_eq!(status.state_description(), "Scanning");
    }

    #[test]
    fn test_parse_eap_status() {
        let response = r#"bssid=aa:bb:cc:dd:ee:ff
freq=5180
ssid=Enterprise
id=1
mode=station
key_mgmt=WPA-EAP
wpa_state=COMPLETED
identity=user@example.com"#;

        let status = SupplicantStatus::parse(response);

        assert!(status.is_connected());
        assert_eq!(status.security, Some(Security::Eap));
        assert_eq!(status.eap_identity, Some("user@example.com".to_string()));
        assert_eq!(status.band(), Some("5 GHz"));
    }

    #[test]
    fn test_associating_states() {
        for state in [
            "ASSOCIATING",
            "ASSOCIATED",
            "4WAY_HANDSHAKE",
            "GROUP_HANDSHAKE",
        ] {
            let response = format!("wpa_state={}", state);
            let status = SupplicantStatus::parse(&response);
            assert!(
                status.is_associating(),
                "state {} should be associating",
                state
            );
        }
    }
}
