//! WiFi operations module
//!
//! This module provides high-level WiFi management functionality including:
//! - Network scanning
//! - Known network management
//! - Connection control
//! - Status queries
mod wpa_supplicant;

use std::path::Path;

use kameo::{Actor, message::StreamMessage, prelude::Message};
use libnetwork_daemon::{
    WiFiManagerAction, WiFiManagerResponse, error::WifiError,
};
pub use wpa_supplicant::*;
pub mod manager;
pub use manager::WifiManager;

/// PSK password minimum length
pub const PSK_MIN_LEN: usize = 8;

/// PSK password maximum length
pub const PSK_MAX_LEN: usize = 63;

/// EAP credential minimum length
pub const EAP_MIN_LEN: usize = 0;

/// EAP credential maximum length
pub const EAP_MAX_LEN: usize = 256;

/// IEEE 802.11 SSID maximum length
pub const IEEE80211_NWID_LEN: usize = 32;

use kameo::actor::ActorRef;

/// WiFi Manager Actor Trait
pub trait WifiManagerBackend:
    Actor<Args = Self>
    + Message<WiFiManagerAction, Reply = Result<WiFiManagerResponse, WifiError>>
    + Message<StreamMessage<WiFiManagerAction, (), ()>>
    + Send
    + Sync
    + 'static
{
    fn new<P, S>(
        workdir: P,
        iface: S,
        supervisor: ActorRef<WifiManager<Self>>,
    ) -> impl Future<Output = Result<Self, WifiError>> + Send
    where
        P: AsRef<Path> + Send + Sync,
        S: AsRef<str> + Send + Sync,
        Self: Sized;
}
