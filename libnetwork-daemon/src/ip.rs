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
        let mask = !0u32 << (32 - self.prefix_len);
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

impl Into<PrefixedIpAddr> for PrefixedIpv4Addr {
    fn into(self) -> PrefixedIpAddr {
        PrefixedIpAddr::V4(self)
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
        let mask = !0u128 << (128 - self.prefix_len);
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

impl Into<PrefixedIpAddr> for PrefixedIpv6Addr {
    fn into(self) -> PrefixedIpAddr {
        PrefixedIpAddr::V6(self)
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
