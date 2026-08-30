use core::fmt;
use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PrefixedIpv4Addr {
    pub addr: std::net::Ipv4Addr,
    pub prefix_len: u8,
}

impl PrefixedIpv4Addr {
    pub const UNSPECIFIED: Self = Self {
        addr: std::net::Ipv4Addr::UNSPECIFIED,
        prefix_len: 0,
    };

    /// Create a new Ipv4Addr
    pub const fn new(addr: std::net::Ipv4Addr, prefix_len: u8) -> Self {
        Self { addr, prefix_len }
    }

    /// Get net id
    pub fn net_id(&self) -> Self {
        // A prefix of 0 (default route) must clear all bits; a plain shift by
        // `32 - 0 = 32` would overflow, so use a checked shift (None -> 0).
        let mask = (!0u32).checked_shl(32 - self.prefix_len as u32).unwrap_or(0);
        let net_id = self.addr.to_bits() & mask;
        Self {
            addr: std::net::Ipv4Addr::from(net_id),
            prefix_len: self.prefix_len,
        }
    }
}

impl fmt::Display for PrefixedIpv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix_len)
    }
}

impl fmt::Debug for PrefixedIpv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Ipv4Addr")
            .field(&format!("{}", self))
            .finish()
    }
}

impl Serialize for PrefixedIpv4Addr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{}", self))
    }
}

impl<'de> Deserialize<'de> for PrefixedIpv4Addr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(serde::de::Error::custom(
                "Invalid IPv4 address format",
            ));
        }
        let addr = parts[0]
            .parse::<std::net::Ipv4Addr>()
            .map_err(|_| serde::de::Error::custom("Invalid IPv4 address"))?;
        let prefix_len = parts[1]
            .parse::<u8>()
            .map_err(|_| serde::de::Error::custom("Invalid prefix length"))?;
        Ok(PrefixedIpv4Addr { addr, prefix_len })
    }
}

impl JsonSchema for PrefixedIpv4Addr {
    fn schema_name() -> Cow<'static, str> {
        "Ipv4Addr".into()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> schemars::Schema {
        json_schema!(
            {
                "type": "string",
                "pattern": r"^(\d{1,3}\.){3}\d{1,3}/\d{1,2}$"
            }
        )
    }
}

impl From<PrefixedIpv4Addr> for PrefixedIpAddr {
    fn from(addr: PrefixedIpv4Addr) -> Self {
        PrefixedIpAddr::V4(addr)
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PrefixedIpv6Addr {
    pub addr: std::net::Ipv6Addr,
    pub prefix_len: u8,
}

impl PrefixedIpv6Addr {
    pub const UNSPECIFIED: Self = Self {
        addr: std::net::Ipv6Addr::UNSPECIFIED,
        prefix_len: 0,
    };

    /// Create a new Ipv6Addr
    pub const fn new(addr: std::net::Ipv6Addr, prefix_len: u8) -> Self {
        Self { addr, prefix_len }
    }

    /// Get net id
    pub fn net_id(&self) -> Self {
        // A prefix of 0 (default route) must clear all bits; a plain shift by
        // `128 - 0 = 128` would overflow, so use a checked shift (None -> 0).
        let mask = (!0u128).checked_shl(128 - self.prefix_len as u32).unwrap_or(0);
        let net_id = self.addr.to_bits() & mask;

        Self {
            addr: std::net::Ipv6Addr::from(net_id),
            prefix_len: self.prefix_len,
        }
    }
}

impl fmt::Display for PrefixedIpv6Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix_len)
    }
}

impl fmt::Debug for PrefixedIpv6Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Ipv6Addr")
            .field(&format!("{}", self))
            .finish()
    }
}

impl Serialize for PrefixedIpv6Addr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{}", self))
    }
}

impl<'de> Deserialize<'de> for PrefixedIpv6Addr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(serde::de::Error::custom(
                "Invalid IPv6 address format",
            ));
        }
        let addr = parts[0]
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| serde::de::Error::custom("Invalid IPv6 address"))?;
        let prefix_len = parts[1]
            .parse::<u8>()
            .map_err(|_| serde::de::Error::custom("Invalid prefix length"))?;
        Ok(PrefixedIpv6Addr { addr, prefix_len })
    }
}

impl From<PrefixedIpv6Addr> for PrefixedIpAddr {
    fn from(addr: PrefixedIpv6Addr) -> Self {
        PrefixedIpAddr::V6(addr)
    }
}

impl JsonSchema for PrefixedIpv6Addr {
    fn schema_name() -> Cow<'static, str> {
        "Ipv6Addr".into()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        json_schema!(
            {
                "type": "string",
                "pattern": r"^([0-9a-fA-F]{0,4}:){2,7}[0-9a-fA-F]{0,4}/\d{1,3}$"
            }
        )
    }
}

/// A Wrapper enum for IPv4 and IPv6 addresses
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PrefixedIpAddr {
    V4(PrefixedIpv4Addr),
    V6(PrefixedIpv6Addr),
}

pub trait IntoPrefixed<T> {
    fn into_prefixed(self, prefix_len: u8) -> T;
}

impl IntoPrefixed<PrefixedIpAddr> for std::net::IpAddr {
    fn into_prefixed(self, prefix_len: u8) -> PrefixedIpAddr {
        match self {
            std::net::IpAddr::V4(v4) => {
                PrefixedIpAddr::V4(PrefixedIpv4Addr::new(v4, prefix_len))
            }
            std::net::IpAddr::V6(v6) => {
                PrefixedIpAddr::V6(PrefixedIpv6Addr::new(v6, prefix_len))
            }
        }
    }
}

impl IntoPrefixed<PrefixedIpv4Addr> for std::net::Ipv4Addr {
    fn into_prefixed(self, prefix_len: u8) -> PrefixedIpv4Addr {
        PrefixedIpv4Addr::new(self, prefix_len)
    }
}

impl IntoPrefixed<PrefixedIpv6Addr> for std::net::Ipv6Addr {
    fn into_prefixed(self, prefix_len: u8) -> PrefixedIpv6Addr {
        PrefixedIpv6Addr::new(self, prefix_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn v4_net_id_zero_prefix() {
        let p = PrefixedIpv4Addr::new(Ipv4Addr::new(192, 168, 1, 5), 0);
        assert_eq!(p.net_id(), PrefixedIpv4Addr::new(Ipv4Addr::UNSPECIFIED, 0));
    }

    #[test]
    fn v4_net_id_masks_host_bits() {
        let p = PrefixedIpv4Addr::new(Ipv4Addr::new(192, 168, 1, 55), 24);
        assert_eq!(
            p.net_id(),
            PrefixedIpv4Addr::new(Ipv4Addr::new(192, 168, 1, 0), 24)
        );
    }

    #[test]
    fn v4_net_id_preserves_classful_high_bits() {
        let p = PrefixedIpv4Addr::new(Ipv4Addr::new(10, 20, 30, 40), 8);
        assert_eq!(
            p.net_id(),
            PrefixedIpv4Addr::new(Ipv4Addr::new(10, 0, 0, 0), 8)
        );
    }

    #[test]
    fn v4_net_id_full_prefix_is_identity() {
        let addr = Ipv4Addr::new(203, 0, 113, 9);
        let p = PrefixedIpv4Addr::new(addr, 32);
        assert_eq!(p.net_id(), p);
    }

    #[test]
    fn v6_net_id_masks_host_bits() {
        let p =
            PrefixedIpv6Addr::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, 1), 64);
        assert_eq!(
            p.net_id(),
            PrefixedIpv6Addr::new(
                Ipv6Addr::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, 0),
                64
            )
        );
    }

    #[test]
    fn v6_net_id_default_route_prefix() {
        let p = PrefixedIpv6Addr::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 0);
        assert_eq!(p.net_id(), PrefixedIpv6Addr::UNSPECIFIED);
    }

    #[test]
    fn v4_unspecified_constant() {
        assert_eq!(
            PrefixedIpv4Addr::UNSPECIFIED,
            PrefixedIpv4Addr::new(Ipv4Addr::UNSPECIFIED, 0)
        );
    }

    #[test]
    fn display_round_trip() {
        let a = PrefixedIpv4Addr::new(Ipv4Addr::new(192, 168, 1, 5), 24);
        assert_eq!(a.to_string(), "192.168.1.5/24");
        let ser = serde_json::to_string(&a).unwrap();
        assert_eq!(ser, "\"192.168.1.5/24\"");
        let de: PrefixedIpv4Addr = serde_json::from_str(&ser).unwrap();
        assert_eq!(de, a);
    }

    #[test]
    fn v6_display_round_trip() {
        let a = PrefixedIpv6Addr::new(
            Ipv6Addr::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, 1),
            64,
        );
        let ser = serde_json::to_string(&a).unwrap();
        let de: PrefixedIpv6Addr = serde_json::from_str(&ser).unwrap();
        assert_eq!(de, a);
    }
}
