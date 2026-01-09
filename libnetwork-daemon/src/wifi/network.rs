//! Network data structures

use bitflags::bitflags;
use schemars::JsonSchema;
use serde::Serialize;

use crate::{MacAddr, utils::unescape_ssid, wifi::Security};

/// Known network state
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, JsonSchema,
)]
pub enum KnownNetworkState {
    /// Network is enabled for auto-connect
    #[default]
    Enabled,
    /// Network is disabled
    Disabled,
    /// Network is disconnected / not connected
    Disconnected,
    /// Currently connected network
    Current,
}

impl KnownNetworkState {
    /// Parse from wpa_supplicant flags
    ///
    /// # Arguments
    ///
    /// * `flags` - Flags string from LIST_NETWORKS like "[CURRENT]" or
    ///   "[DISABLED]"
    pub fn from_flags(flags: &str) -> Self {
        if flags.contains("CURRENT") {
            Self::Current
        } else if flags.contains("DISABLED") {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }
}

/// A saved/known network
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct KnownNetwork {
    /// Network ID (wpa_supplicant-assigned). Only present after we add the
    /// network into wpa_supplicant during connect.
    #[serde(skip)]
    pub id: Option<i32>,
    /// Network priority (higher = preferred)
    pub priority: i32,
    /// Whether this is a hidden network
    pub hidden: bool,
    /// Security type
    pub security: Security,
    /// Network state
    pub state: KnownNetworkState,
    /// SSID
    pub ssid: String,
    /// BSSID (if configured)
    pub bssid: Option<MacAddr>,
    /// Credential for PSK networks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Identity for EAP networks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

impl KnownNetwork {
    /// Parse from LIST_NETWORKS response line
    ///
    /// Format: `network_id / ssid / bssid / flags`
    ///
    /// # Arguments
    ///
    /// * `line` - Single line from LIST_NETWORKS response
    ///
    /// # Returns
    ///
    /// * `Some(KnownNetwork)` - Successfully parsed
    /// * `None` - Failed to parse (header line or invalid format)
    pub fn parse(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            return None;
        }

        let id: i32 = parts[0].parse().ok()?;
        let ssid = unescape_ssid(parts[1]);
        let bssid = if parts[2] == "any" {
            None
        } else {
            parts[2].parse().ok()
        };

        let state = if parts.len() >= 4 {
            let flags = parts[3];
            KnownNetworkState::from_flags(flags)
        } else {
            KnownNetworkState::Disconnected
        };

        Some(Self {
            id: Some(id),
            priority: 0,
            hidden: false,
            security: Security::Unknown,
            state,
            ssid,
            bssid,
            password: None,
            identity: None,
        })
    }

    /// Check if this is the currently connected network
    pub fn is_current(&self) -> bool {
        self.state == KnownNetworkState::Current
    }

    /// Check if this network is enabled
    pub fn is_enabled(&self) -> bool {
        !matches!(self.state, KnownNetworkState::Disabled)
    }
}

bitflags! {
    /// WiFi capability flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ScanFlags: u32 {
        const WPA = 1 << 0;
        const WPA2 = 1 << 1;
        const WPA3 = 1 << 2;
        const WEP = 1 << 3;
        const ESS = 1 << 4;
        const IBSS = 1 << 5;
        const WPS = 1 << 6;
        const P2P = 1 << 7;
        const ENTERPRISE = 1 << 8;
        const PERSONAL = 1 << 9;
        const CCMP = 1 << 10;
        const TKIP = 1 << 11;
    }
}

impl Serialize for ScanFlags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u32(self.bits())
    }
}

impl JsonSchema for ScanFlags {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("ScanFlags")
    }

    fn json_schema(
        generator: &mut schemars::SchemaGenerator,
    ) -> schemars::Schema {
        u32::json_schema(generator)
    }
}

impl ScanFlags {
    pub fn parse(flags_str: &str) -> Self {
        let mut flags = Self::empty();
        let upper = flags_str.to_uppercase();

        if upper.contains("WPA-") {
            flags.insert(Self::WPA);
        }
        if upper.contains("WPA2-") || upper.contains("RSN-") {
            flags.insert(Self::WPA2);
        }
        if upper.contains("SAE") {
            flags.insert(Self::WPA3);
        }
        if upper.contains("WEP") {
            flags.insert(Self::WEP);
        }

        if upper.contains("[ESS]") {
            flags.insert(Self::ESS);
        }
        if upper.contains("[IBSS]") {
            flags.insert(Self::IBSS);
        }
        if upper.contains("[WPS]") {
            flags.insert(Self::WPS);
        }
        if upper.contains("[P2P]") {
            flags.insert(Self::P2P);
        }

        if upper.contains("EAP") {
            flags.insert(Self::ENTERPRISE);
        }
        if upper.contains("PSK") {
            flags.insert(Self::PERSONAL);
        }

        if upper.contains("CCMP") {
            flags.insert(Self::CCMP);
        }
        if upper.contains("TKIP") {
            flags.insert(Self::TKIP);
        }

        flags
    }
}

/// A scan result (available network)
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScanResult {
    /// Frequency in MHz
    pub freq: i32,
    /// Signal strength in dBm
    pub signal: i32,
    /// BSSID
    pub bssid: MacAddr,
    /// SSID
    pub ssid: String,
    /// Security type
    pub security: Security,
    /// Parsed flags from wpa_supplicant
    pub flags: ScanFlags,
}

impl ScanResult {
    /// Parse from SCAN_RESULTS response line
    ///
    /// Format: `bssid / frequency / signal level / flags / ssid`
    ///
    /// # Arguments
    ///
    /// * `line` - Single line from SCAN_RESULTS response
    ///
    /// # Returns
    ///
    /// * `Some(ScanResult)` - Successfully parsed
    /// * `None` - Failed to parse (header line or invalid format)
    pub fn parse(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 5 {
            return None;
        }

        let bssid: MacAddr = parts[0].parse().ok()?;
        let freq: i32 = parts[1].parse().ok()?;
        let signal: i32 = parts[2].parse().ok()?;
        let flags_str = parts[3];
        let flags = ScanFlags::parse(flags_str);
        let ssid = if parts.len() > 4 {
            unescape_ssid(parts[4])
        } else {
            String::new()
        };

        let security = Security::from_flags(flags_str);

        Some(Self {
            freq,
            signal,
            bssid,
            ssid,
            security,
            flags,
        })
    }

    /// Get signal quality as percentage (0-100)
    ///
    /// Converts dBm to percentage using typical range:
    /// - -30 dBm = 100% (excellent)
    /// - -90 dBm = 0% (unusable)
    pub fn signal_quality(&self) -> u8 {
        let clamped = self.signal.clamp(-90, -30);
        ((clamped + 90) * 100 / 60) as u8
    }

    /// Get WiFi channel number
    pub fn channel(&self) -> u8 {
        match self.freq {
            // 2.4 GHz band
            2412 => 1,
            2417 => 2,
            2422 => 3,
            2427 => 4,
            2432 => 5,
            2437 => 6,
            2442 => 7,
            2447 => 8,
            2452 => 9,
            2457 => 10,
            2462 => 11,
            2467 => 12,
            2472 => 13,
            2484 => 14,
            // 5 GHz band (common channels)
            5180 => 36,
            5200 => 40,
            5220 => 44,
            5240 => 48,
            5260 => 52,
            5280 => 56,
            5300 => 60,
            5320 => 64,
            5500 => 100,
            5520 => 104,
            5540 => 108,
            5560 => 112,
            5580 => 116,
            5600 => 120,
            5620 => 124,
            5640 => 128,
            5660 => 132,
            5680 => 136,
            5700 => 140,
            5720 => 144,
            5745 => 149,
            5765 => 153,
            5785 => 157,
            5805 => 161,
            5825 => 165,
            _ => 0,
        }
    }
}

/// Sort scan results by signal strength (descending)
pub fn sort_by_signal(results: &mut [ScanResult]) {
    results.sort_by(|a, b| b.signal.cmp(&a.signal));
}

/// Sort known networks by priority (descending)
pub fn sort_by_priority(networks: &mut [KnownNetwork]) {
    networks.sort_by(|a, b| b.priority.cmp(&a.priority));
}

/// Filter scan results by SSID (case-insensitive substring match)
pub fn filter_by_ssid(
    results: Vec<ScanResult>,
    filter: &str,
) -> Vec<ScanResult> {
    let filter_lower = filter.to_lowercase();
    results
        .into_iter()
        .filter(|r| r.ssid.to_lowercase().contains(&filter_lower))
        .collect()
}

/// Filter known networks by SSID (case-insensitive substring match)
pub fn filter_known_by_ssid(
    networks: Vec<KnownNetwork>,
    filter: &str,
) -> Vec<KnownNetwork> {
    let filter_lower = filter.to_lowercase();
    networks
        .into_iter()
        .filter(|n| n.ssid.to_lowercase().contains(&filter_lower))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_scan_flags() {
        let flags = ScanFlags::parse("[WPA2-PSK-CCMP][ESS]");
        assert!(flags.contains(ScanFlags::WPA2));
        assert!(flags.contains(ScanFlags::PERSONAL));
        assert!(flags.contains(ScanFlags::CCMP));
        assert!(flags.contains(ScanFlags::ESS));
        assert!(!flags.contains(ScanFlags::WPA));
        assert!(!flags.contains(ScanFlags::WEP));

        let flags = ScanFlags::parse("[WPA2-PSK+SAE-CCMP][SAE-H2E][ESS]");
        assert!(flags.contains(ScanFlags::WPA2));
        assert!(flags.contains(ScanFlags::WPA3));
        assert!(flags.contains(ScanFlags::PERSONAL));
        assert!(flags.contains(ScanFlags::CCMP));
        assert!(flags.contains(ScanFlags::ESS));

        let flags = ScanFlags::parse("[WPA-PSK-CCMP][WPA2-PSK-CCMP][WPS][ESS]");
        assert!(flags.contains(ScanFlags::WPA));
        assert!(flags.contains(ScanFlags::WPA2));
        assert!(flags.contains(ScanFlags::PERSONAL));
        assert!(flags.contains(ScanFlags::CCMP));
        assert!(flags.contains(ScanFlags::WPS));
        assert!(flags.contains(ScanFlags::ESS));
    }

    #[test]
    fn test_parse_scan_result() {
        let line =
            "aa:bb:cc:dd:ee:ff\t2412\t-50\t[WPA2-PSK-CCMP][ESS]\tTestNetwork";
        let result = ScanResult::parse(line).unwrap();

        assert_eq!(
            result.bssid.to_string().to_lowercase(),
            "aa:bb:cc:dd:ee:ff"
        );
        assert_eq!(result.freq, 2412);
        assert_eq!(result.signal, -50);
        assert_eq!(result.ssid, "TestNetwork");
        assert_eq!(result.security, Security::Psk);
        assert_eq!(result.channel(), 1);
    }

    #[test]
    fn test_parse_scan_result_5ghz() {
        let line =
            "11:22:33:44:55:66\t5180\t-65\t[WPA2-EAP-CCMP][ESS]\tEnterprise";
        let result = ScanResult::parse(line).unwrap();

        assert_eq!(result.freq, 5180);
        assert_eq!(result.security, Security::Eap);
        assert_eq!(result.channel(), 36);
    }

    #[test]
    fn test_parse_scan_result_open() {
        let line = "aa:bb:cc:dd:ee:ff\t2437\t-70\t[ESS]\tOpenNetwork";
        let result = ScanResult::parse(line).unwrap();

        assert_eq!(result.security, Security::Open);
    }

    #[test]
    fn test_signal_quality() {
        let mut result = ScanResult {
            freq: 2412,
            signal: -30,
            bssid: "aa:bb:cc:dd:ee:ff".parse().unwrap(),
            ssid: "Test".to_string(),
            security: Security::Open,
            flags: ScanFlags::default(),
        };

        assert_eq!(result.signal_quality(), 100);

        result.signal = -90;
        assert_eq!(result.signal_quality(), 0);

        result.signal = -60;
        assert_eq!(result.signal_quality(), 50);
    }

    #[test]
    fn test_parse_known_network() {
        let line = "0\tMyNetwork\tany\t[CURRENT]";
        let network = KnownNetwork::parse(line).unwrap();

        assert_eq!(network.id, Some(0));
        assert_eq!(network.ssid, "MyNetwork");
        assert!(network.bssid.is_none());
        assert_eq!(network.state, KnownNetworkState::Current);
        assert!(network.is_current());
    }

    #[test]
    fn test_parse_known_network_disabled() {
        let line = "1\tOtherNetwork\taa:bb:cc:dd:ee:ff\t[DISABLED]";
        let network = KnownNetwork::parse(line).unwrap();

        assert_eq!(network.id, Some(1));
        assert_eq!(network.state, KnownNetworkState::Disabled);
        assert!(!network.is_enabled());
        assert!(network.bssid.is_some());
    }

    #[test]
    fn test_sort_by_signal() {
        let mut results = vec![
            ScanResult {
                freq: 2412,
                signal: -70,
                bssid: "aa:bb:cc:dd:ee:ff".parse().unwrap(),
                ssid: "Weak".to_string(),
                security: Security::Open,
                flags: ScanFlags::default(),
            },
            ScanResult {
                freq: 2412,
                signal: -50,
                bssid: "11:22:33:44:55:66".parse().unwrap(),
                ssid: "Strong".to_string(),
                security: Security::Open,
                flags: ScanFlags::default(),
            },
        ];

        sort_by_signal(&mut results);
        assert_eq!(results[0].ssid, "Strong");
        assert_eq!(results[1].ssid, "Weak");
    }

    #[test]
    fn test_filter_by_ssid() {
        let results = vec![
            ScanResult {
                freq: 2412,
                signal: -50,
                bssid: "aa:bb:cc:dd:ee:ff".parse().unwrap(),
                ssid: "TestNetwork".to_string(),
                security: Security::Open,
                flags: ScanFlags::default(),
            },
            ScanResult {
                freq: 2412,
                signal: -60,
                bssid: "11:22:33:44:55:66".parse().unwrap(),
                ssid: "OtherNet".to_string(),
                security: Security::Open,
                flags: ScanFlags::default(),
            },
        ];

        let filtered = filter_by_ssid(results, "test");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].ssid, "TestNetwork");
    }
}
