//! WPA Supplicant event constants and parsing

/// Authentication completed, data connection enabled
pub const WPA_EVENT_CONNECTED: &str = "CTRL-EVENT-CONNECTED";

/// Disconnected from network
pub const WPA_EVENT_DISCONNECTED: &str = "CTRL-EVENT-DISCONNECTED";

/// Association rejected when connecting
pub const WPA_EVENT_ASSOC_REJECT: &str = "CTRL-EVENT-ASSOC-REJECT";

/// Authentication rejected when connecting  
pub const WPA_EVENT_AUTH_REJECT: &str = "CTRL-EVENT-AUTH-REJECT";

/// wpa_supplicant is exiting
pub const WPA_EVENT_TERMINATING: &str = "CTRL-EVENT-TERMINATING";

/// Scan has started
pub const WPA_EVENT_SCAN_STARTED: &str = "CTRL-EVENT-SCAN-STARTED";

/// New scan results available
pub const WPA_EVENT_SCAN_RESULTS: &str = "CTRL-EVENT-SCAN-RESULTS";

/// Scan failed
pub const WPA_EVENT_SCAN_FAILED: &str = "CTRL-EVENT-SCAN-FAILED";

/// State change notification
pub const WPA_EVENT_STATE_CHANGE: &str = "CTRL-EVENT-STATE-CHANGE";

/// No suitable network found
pub const WPA_EVENT_NETWORK_NOT_FOUND: &str = "CTRL-EVENT-NETWORK-NOT-FOUND";

/// Network temporarily disabled
pub const WPA_EVENT_TEMP_DISABLED: &str = "CTRL-EVENT-SSID-TEMP-DISABLED";

/// Re-enabled after temp disable
pub const WPA_EVENT_REENABLED: &str = "CTRL-EVENT-SSID-REENABLED";

/// Associated with AP (custom event marker)
pub const WPA_EVENT_ASSOCIATED: &str = "Associated with";

/// EAP events
pub const WPA_EVENT_EAP_STARTED: &str = "CTRL-EVENT-EAP-STARTED";
pub const WPA_EVENT_EAP_METHOD: &str = "CTRL-EVENT-EAP-METHOD";
pub const WPA_EVENT_EAP_SUCCESS: &str = "CTRL-EVENT-EAP-SUCCESS";
pub const WPA_EVENT_EAP_FAILURE: &str = "CTRL-EVENT-EAP-FAILURE";

/// BSS mask flags for filtering BSS query results
pub const WPA_BSS_MASK_ID: u32 = 1 << 0;
pub const WPA_BSS_MASK_BSSID: u32 = 1 << 1;
pub const WPA_BSS_MASK_FREQ: u32 = 1 << 2;
pub const WPA_BSS_MASK_BEACON_INT: u32 = 1 << 3;
pub const WPA_BSS_MASK_CAPABILITIES: u32 = 1 << 4;
pub const WPA_BSS_MASK_QUAL: u32 = 1 << 5;
pub const WPA_BSS_MASK_NOISE: u32 = 1 << 6;
pub const WPA_BSS_MASK_LEVEL: u32 = 1 << 7;
pub const WPA_BSS_MASK_TSF: u32 = 1 << 8;
pub const WPA_BSS_MASK_AGE: u32 = 1 << 9;
pub const WPA_BSS_MASK_IE: u32 = 1 << 10;
pub const WPA_BSS_MASK_FLAGS: u32 = 1 << 11;
pub const WPA_BSS_MASK_SSID: u32 = 1 << 12;
pub const WPA_BSS_MASK_WPS_SCAN: u32 = 1 << 13;
pub const WPA_BSS_MASK_P2P_SCAN: u32 = 1 << 14;
pub const WPA_BSS_MASK_INTERNETW: u32 = 1 << 15;
pub const WPA_BSS_MASK_WIFI_DISPLAY: u32 = 1 << 16;
pub const WPA_BSS_MASK_DELIM: u32 = 1 << 17;
pub const WPA_BSS_MASK_MESH_SCAN: u32 = 1 << 18;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WpaTempDisabledReason {
    AuthFailure,
    AssociationFailure,
    NoNetwork,
    Unknown(String),
}

impl WpaTempDisabledReason {
    pub fn parse(reason_str: &str) -> Self {
        match reason_str {
            "AUTH_FAILURE" | "WRONG_KEY" => WpaTempDisabledReason::AuthFailure,
            "ASSOCIATION_FAILURE" => WpaTempDisabledReason::AssociationFailure,
            "NO_NETWORK" => WpaTempDisabledReason::NoNetwork,
            other => WpaTempDisabledReason::Unknown(other.to_string()),
        }
    }
}

/// Parsed WPA event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WpaEvent {
    /// Connected to network
    Connected { bssid: String },
    /// Disconnected from network
    Disconnected {
        bssid: Option<String>,
        reason: Option<i32>,
    },
    /// Scan results available
    ScanResults,
    /// Scan started
    ScanStarted,
    /// Scan failed
    ScanFailed,
    /// State changed
    StateChange { old: String, new: String },
    /// Network not found
    NetworkNotFound,
    /// Network temporarily disabled
    TempDisabled {
        ssid: String,
        reason: WpaTempDisabledReason,
    },
    /// Associated with AP
    Associated { bssid: String },
    /// BSS removed from scan results
    BssRemoved { bssid: String },
    /// Command success
    Ok,
    /// Unknown event
    Unknown(String),
}

impl WpaEvent {
    /// Parse a raw event string from wpa_supplicant
    ///
    /// # Arguments
    ///
    /// * `raw` - Raw event string
    ///
    /// # Returns
    ///
    /// Parsed `WpaEvent` variant
    pub fn parse(raw: &str) -> Self {
        // Strip priority prefix if present (e.g., "<3>")
        let msg = if raw.starts_with('<') {
            if let Some(end) = raw.find('>') {
                &raw[end + 1..]
            } else {
                raw
            }
        } else {
            raw
        };

        if msg == "OK" {
            WpaEvent::Ok
        } else if msg.starts_with("CTRL-EVENT-BSS-REMOVED") {
            // Parse: CTRL-EVENT-BSS-REMOVED 117 78:44:fd:4d:c2:b2
            let bssid = msg.split_whitespace().last().unwrap_or("").to_string();
            WpaEvent::BssRemoved { bssid }
        } else if msg.starts_with(WPA_EVENT_CONNECTED) {
            // Parse: CTRL-EVENT-CONNECTED - Connection to XX:XX:XX:XX:XX:XX
            // completed
            let bssid = msg
                .split("Connection to ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            WpaEvent::Connected { bssid }
        } else if msg.starts_with(WPA_EVENT_DISCONNECTED) {
            // Parse: CTRL-EVENT-DISCONNECTED bssid=XX:XX:XX:XX:XX:XX reason=X
            let bssid = msg
                .split("bssid=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .map(|s| s.to_string());
            let reason = msg
                .split("reason=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok());
            WpaEvent::Disconnected { bssid, reason }
        } else if msg.starts_with(WPA_EVENT_SCAN_RESULTS) {
            WpaEvent::ScanResults
        } else if msg.starts_with(WPA_EVENT_SCAN_STARTED) {
            WpaEvent::ScanStarted
        } else if msg.starts_with(WPA_EVENT_SCAN_FAILED) {
            WpaEvent::ScanFailed
        } else if msg.starts_with(WPA_EVENT_STATE_CHANGE) {
            // Parse: CTRL-EVENT-STATE-CHANGE id=X state=Y ...
            let old = msg
                .split("old_state=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            // The new state is under `state=` (not `new_state=`).
            let new = msg
                .split("state=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            WpaEvent::StateChange { old, new }
        } else if msg.starts_with(WPA_EVENT_NETWORK_NOT_FOUND) {
            WpaEvent::NetworkNotFound
        } else if msg.starts_with(WPA_EVENT_TEMP_DISABLED) {
            // Parse: CTRL-EVENT-SSID-TEMP-DISABLED id=X ssid="Y"
            // auth_failures=Z duration=W reason=R
            let ssid = msg
                .split("ssid=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();
            let reason = msg
                .split("reason=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            let reason = WpaTempDisabledReason::parse(&reason);
            WpaEvent::TempDisabled { ssid, reason }
        } else if msg.contains(WPA_EVENT_ASSOCIATED) {
            // Parse: Associated with XX:XX:XX:XX:XX:XX
            let bssid = msg
                .split(WPA_EVENT_ASSOCIATED)
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            WpaEvent::Associated { bssid }
        } else {
            WpaEvent::Unknown(raw.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_connected() {
        let raw = "<3>CTRL-EVENT-CONNECTED - Connection to aa:bb:cc:dd:ee:ff \
                   completed [id=0]";
        let event = WpaEvent::parse(raw);
        assert_eq!(
            event,
            WpaEvent::Connected {
                bssid: "aa:bb:cc:dd:ee:ff".to_string()
            }
        );
    }

    #[test]
    fn test_parse_disconnected() {
        let raw = "<3>CTRL-EVENT-DISCONNECTED bssid=aa:bb:cc:dd:ee:ff reason=3";
        let event = WpaEvent::parse(raw);
        assert_eq!(
            event,
            WpaEvent::Disconnected {
                bssid: Some("aa:bb:cc:dd:ee:ff".to_string()),
                reason: Some(3)
            }
        );
    }

    #[test]
    fn test_parse_scan_results() {
        let raw = "<3>CTRL-EVENT-SCAN-RESULTS";
        let event = WpaEvent::parse(raw);
        assert_eq!(event, WpaEvent::ScanResults);
    }

    #[test]
    fn test_parse_temp_disabled() {
        let raw = "<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid=\"TestNetwork\" \
                   auth_failures=3 duration=30 reason=WRONG_KEY";
        let event = WpaEvent::parse(raw);
        assert_eq!(
            event,
            WpaEvent::TempDisabled {
                ssid: "TestNetwork".to_string(),
                reason: WpaTempDisabledReason::AuthFailure
            }
        );
    }

    #[test]
    fn test_parse_associated() {
        let raw = "Associated with aa:bb:cc:dd:ee:ff";
        let event = WpaEvent::parse(raw);
        assert_eq!(
            event,
            WpaEvent::Associated {
                bssid: "aa:bb:cc:dd:ee:ff".to_string()
            }
        );
    }

    #[test]
    fn test_parse_unknown() {
        let raw = "SOME-UNKNOWN-EVENT data";
        let event = WpaEvent::parse(raw);
        assert_eq!(
            event,
            WpaEvent::Unknown("SOME-UNKNOWN-EVENT data".to_string())
        );
    }

    #[test]
    fn test_parse_bss_removed() {
        let raw = "<3>CTRL-EVENT-BSS-REMOVED 117 78:44:fd:4d:c2:b2";
        assert_eq!(
            WpaEvent::parse(raw),
            WpaEvent::BssRemoved {
                bssid: "78:44:fd:4d:c2:b2".to_string()
            }
        );
    }

    #[test]
    fn test_parse_state_change() {
        let raw =
            "<3>CTRL-EVENT-STATE-CHANGE id=0 state=COMPLETED old_state=SCANNING";
        assert_eq!(
            WpaEvent::parse(raw),
            WpaEvent::StateChange {
                old: "SCANNING".to_string(),
                new: "COMPLETED".to_string()
            }
        );
    }

    #[test]
    fn test_parse_disconnected_without_reason() {
        let raw = "<2>CTRL-EVENT-DISCONNECTED bssid=aa:bb:cc:dd:ee:ff";
        assert_eq!(
            WpaEvent::parse(raw),
            WpaEvent::Disconnected {
                bssid: Some("aa:bb:cc:dd:ee:ff".to_string()),
                reason: None
            }
        );
    }

    #[test]
    fn test_parse_scan_started_and_failed() {
        assert_eq!(
            WpaEvent::parse("CTRL-EVENT-SCAN-STARTED"),
            WpaEvent::ScanStarted
        );
        assert_eq!(
            WpaEvent::parse("CTRL-EVENT-SCAN-FAILED ret=-1"),
            WpaEvent::ScanFailed
        );
    }

    #[test]
    fn test_parse_ok() {
        assert_eq!(WpaEvent::parse("OK"), WpaEvent::Ok);
    }

    #[test]
    fn test_parse_priority_less_connected() {
        // Some wpa_supplicant builds omit the <N> priority prefix.
        let raw = "CTRL-EVENT-CONNECTED - Connection to 11:22:33:44:55:66 completed";
        assert_eq!(
            WpaEvent::parse(raw),
            WpaEvent::Connected {
                bssid: "11:22:33:44:55:66".to_string()
            }
        );
    }

    #[test]
    fn test_parse_temp_disabled_unknown_reason() {
        let raw =
            "<3>CTRL-EVENT-SSID-TEMP-DISABLED id=0 ssid=\"X\" reason=NOPE";
        assert_eq!(
            WpaEvent::parse(raw),
            WpaEvent::TempDisabled {
                ssid: "X".to_string(),
                reason: WpaTempDisabledReason::Unknown("NOPE".to_string())
            }
        );
    }
}
