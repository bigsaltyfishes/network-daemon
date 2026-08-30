use std::{
    fmt::{self, Debug},
    str::FromStr,
};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddr([u8; 6]);

impl MacAddr {
    pub const fn new(bytes: [u8; 6]) -> MacAddr {
        MacAddr(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    pub fn parse<T: AsRef<str>>(s: T) -> Option<Self> {
        let s = s.as_ref();
        let s = s.trim().to_ascii_lowercase();
        s.split(":")
            .map(|part| u8::from_str_radix(part, 16).ok())
            .collect::<Option<Vec<u8>>>()
            .and_then(|bytes| {
                if bytes.len() == 6 {
                    let mut arr = [0u8; 6];
                    arr.copy_from_slice(&bytes);
                    Some(MacAddr(arr))
                } else {
                    None
                }
            })
    }
}

impl FromStr for MacAddr {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        MacAddr::parse(s).ok_or(())
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl Debug for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("MacAddr")
            .field(&format!("{}", self))
            .finish()
    }
}

impl Serialize for MacAddr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{}", self))
    }
}

impl<'de> Deserialize<'de> for MacAddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        MacAddr::parse(&s).ok_or_else(|| {
            serde::de::Error::custom("Invalid MAC address format")
        })
    }
}

impl JsonSchema for MacAddr {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "MacAddr".into()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        json_schema!(
            {
                "type": "string",
                "pattern": r"^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_lowercase() {
        let mac = MacAddr::parse("aa:bb:cc:dd:ee:ff").unwrap();
        assert_eq!(mac.to_string(), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn parse_is_case_insensitive_and_trimmed() {
        let mac = MacAddr::parse("  AA:BB:CC:DD:EE:FF ").unwrap();
        assert_eq!(mac.to_string(), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn parse_rejects_bad_length_and_format() {
        assert!(MacAddr::parse("aa:bb:cc:dd:ee").is_none());
        assert!(MacAddr::parse("aa:bb:cc:dd:ee:ff:00").is_none());
        assert!(MacAddr::parse("zz:bb:cc:dd:ee:ff").is_none());
        assert!(MacAddr::parse("").is_none());
    }

    #[test]
    fn serde_round_trip() {
        let mac = MacAddr::parse("10:20:30:40:50:60").unwrap();
        let ser = serde_json::to_string(&mac).unwrap();
        assert_eq!(ser, "\"10:20:30:40:50:60\"");
        let de: MacAddr = serde_json::from_str(&ser).unwrap();
        assert_eq!(de, mac);
    }

    #[test]
    fn bytes_round_trip() {
        let mac = MacAddr::new([1, 2, 3, 4, 5, 6]);
        assert_eq!(mac.as_bytes(), &[1, 2, 3, 4, 5, 6]);
    }
}
