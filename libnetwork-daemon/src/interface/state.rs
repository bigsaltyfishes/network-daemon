//! Network connection state

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
pub enum ConnectedState {
    /// Connected to the Internet
    Internet,
    /// Connected to a local network only
    #[default]
    Local,
}

impl fmt::Display for ConnectedState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Internet => write!(f, "Connected"),
            Self::Local => write!(f, "Local Only"),
        }
    }
}

/// Network connection state
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
pub enum ConnectionState {
    /// Interface is active
    Up,
    /// Connected (has IP address)
    Connected,
    /// Disconnected (interface up but no connection)
    Disconnected,
    /// No carrier (physically disconnected or no carrier)
    NoCarrier,
    /// Disabled (interface down)
    Disabled,
    /// Not applicable / unknown
    #[default]
    NotApplicable,
    /// Unmanaged (not managed by the network daemon)
    Unmanaged,
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Up => write!(f, "Up"),
            Self::Connected => write!(f, "Connected"),
            Self::Disconnected => write!(f, "Disconnected"),
            Self::NoCarrier => write!(f, "Unplugged"),
            Self::Disabled => write!(f, "Disabled"),
            Self::NotApplicable => write!(f, "N/A"),
            Self::Unmanaged => write!(f, "Unmanaged"),
        }
    }
}

impl ConnectionState {
    /// Check if online (Connected)
    pub fn is_online(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Check if available for connection attempts
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Connected | Self::Disconnected)
    }

    /// Check if the interface is up
    pub fn is_up(&self) -> bool {
        matches!(self, Self::Up | Self::Connected | Self::Disconnected)
    }

    /// Check if the interface is managed
    pub fn is_managed(&self) -> bool {
        !matches!(self, Self::Unmanaged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_display() {
        assert_eq!(ConnectionState::Connected.to_string(), "Connected");
        assert_eq!(ConnectionState::Disconnected.to_string(), "Disconnected");
        assert_eq!(ConnectionState::NoCarrier.to_string(), "Unplugged");
        assert_eq!(ConnectionState::Disabled.to_string(), "Disabled");
        assert_eq!(ConnectionState::NotApplicable.to_string(), "N/A");
    }

    #[test]
    fn test_connection_state_predicates() {
        assert!(ConnectionState::Connected.is_online());
        assert!(!ConnectionState::Disconnected.is_online());

        assert!(ConnectionState::Connected.is_available());
        assert!(ConnectionState::Disconnected.is_available());
        assert!(!ConnectionState::Disabled.is_available());

        assert!(ConnectionState::Connected.is_up());
        assert!(ConnectionState::Disconnected.is_up());
        assert!(!ConnectionState::Disabled.is_up());
    }

    #[test]
    fn test_default() {
        assert_eq!(ConnectionState::default(), ConnectionState::NotApplicable);
    }
}
